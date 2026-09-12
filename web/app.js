(() => {
	"use strict";

	const elements = {
		app: document.querySelector("#app"),
		menuToggle: document.querySelector("#menu-toggle"),
		sessionMenu: document.querySelector("#session-menu"),
		sessionList: document.querySelector("#session-list"),
		newSession: document.querySelector("#new-session"),
		title: document.querySelector("#session-title"),
		conversation: document.querySelector("#conversation"),
		messageList: document.querySelector("#message-list"),
		empty: document.querySelector("#empty-state"),
		timeline: document.querySelector("#timeline"),
		timelineList: document.querySelector("#timeline-list"),
		prompt: document.querySelector("#prompt"),
		send: document.querySelector("#send"),
		toast: document.querySelector("#toast"),
	};

	const state = {
		ws: null,
		connected: false,
		reconnectAttempt: 0,
		sessions: [],
		active: null,
		items: [],
		entries: [],
		streaming: false,
		streamingIndex: -1,
		tools: new Map(),
		autoOpenDone: false,
		toastTimer: null,
	};

	function connect() {
		const scheme = location.protocol === "https:" ? "wss" : "ws";
		const ws = new WebSocket(`${scheme}://${location.host}/ws`);
		state.ws = ws;

		ws.addEventListener("open", () => {
			state.connected = true;
			state.reconnectAttempt = 0;
			setEmptyStatus("选择或新建 session");
			send({ type: "list_sessions" });
		});

		ws.addEventListener("message", (event) => {
			try {
				handleServer(JSON.parse(event.data));
			} catch (error) {
				console.error("Invalid server message", error);
			}
		});

		ws.addEventListener("close", () => {
			state.connected = false;
			setEmptyStatus("连接已断开，正在重连");
			const delay = Math.min(10_000, 500 * 2 ** state.reconnectAttempt++);
			window.setTimeout(connect, delay);
		});

		ws.addEventListener("error", () => ws.close());
	}

	function send(payload) {
		if (!state.ws || state.ws.readyState !== WebSocket.OPEN) {
			showToast("连接尚未恢复");
			return false;
		}
		state.ws.send(JSON.stringify(payload));
		return true;
	}

	function handleServer(message) {
		switch (message.type) {
			case "hello":
				if (message.protocol !== 1) showToast("前后端协议版本不一致");
				break;
			case "sessions":
				state.sessions = message.sessions || [];
				if (state.active?.runtime_id) {
					const current = state.sessions.find(
						(session) => session.runtime_id === state.active.runtime_id,
					);
					if (current) {
						state.active = current;
						elements.title.textContent = current.title || "新对话";
						localStorage.setItem("pi-mobile-catalog", current.catalog_id);
					}
				}
				renderSessions();
				autoOpenSession();
				break;
			case "session_opened":
				openSessionPayload(message);
				break;
			case "rpc_event":
				if (message.runtime_id === state.active?.runtime_id) handleRpc(message.event);
				break;
			case "error":
				showToast(message.message || "发生错误");
				break;
		}
	}

	function autoOpenSession() {
		if (state.active || state.autoOpenDone || !state.connected) return;
		state.autoOpenDone = true;
		const remembered = localStorage.getItem("pi-mobile-catalog");
		const target = state.sessions.find((session) => session.catalog_id === remembered) || state.sessions[0];
		if (!target) {
			send({ type: "create_session" });
			return;
		}
		requestSession(target);
	}

	function requestSession(session) {
		if (session.runtime_id) {
			send({ type: "activate_session", runtime_id: session.runtime_id });
		} else {
			send({ type: "open_session", catalog_id: session.catalog_id });
		}
	}

	function openSessionPayload(payload) {
		state.active = payload.session;
		state.items = (payload.messages || []).map((message) => ({ kind: "message", message }));
		state.entries = payload.entries || [];
		state.streaming = Boolean(payload.state?.isStreaming);
		state.streamingIndex = -1;
		state.tools.clear();
		elements.title.textContent = payload.session.title || "新对话";
		localStorage.setItem("pi-mobile-catalog", payload.session.catalog_id);
		closeMenu();
		closeTimeline();
		renderMessages(false);
		renderTimeline();
		renderSessions();
		updateSendButton();
	}

	function handleRpc(event) {
		switch (event?.type) {
			case "agent_start":
				state.streaming = true;
				updateSendButton();
				break;
			case "agent_settled":
				state.streaming = false;
				state.streamingIndex = -1;
				updateSendButton();
				send({ type: "list_sessions" });
				break;
			case "agent_end":
				if (!event.willRetry) {
					state.streaming = false;
					updateSendButton();
				}
				break;
			case "message_start":
				handleMessageStart(event.message);
				break;
			case "message_update":
				handleMessageDelta(event.assistantMessageEvent);
				break;
			case "message_end":
				handleMessageEnd(event.message);
				break;
			case "tool_execution_start":
				startTool(event);
				break;
			case "tool_execution_update":
				updateTool(event, false);
				break;
			case "tool_execution_end":
				updateTool(event, true);
				break;
			case "compaction_start":
				showToast("正在压缩上下文");
				break;
			case "extension_error":
				showToast(event.error || "Pi 扩展发生错误");
				break;
		}
	}

	function handleMessageStart(message) {
		if (!message) return;
		if (message.role !== "user" && message.role !== "assistant") return;
		const item = { kind: "message", message: structuredClone(message) };
		state.items.push(item);
		if (message.role === "assistant") state.streamingIndex = state.items.length - 1;
		if (message.role === "user") {
			state.entries.push({ type: "message", timestamp: new Date().toISOString(), message });
			renderTimeline();
		}
		renderMessages(true);
	}

	function handleMessageDelta(delta) {
		if (!delta) return;
		if (state.streamingIndex < 0 || !state.items[state.streamingIndex]) {
			state.items.push({ kind: "message", message: { role: "assistant", content: [] } });
			state.streamingIndex = state.items.length - 1;
		}
		const message = state.items[state.streamingIndex].message;
		if (!Array.isArray(message.content)) message.content = [];
		const index = Number.isInteger(delta.contentIndex) ? delta.contentIndex : message.content.length;
		while (message.content.length <= index) message.content.push(null);

		switch (delta.type) {
			case "text_start":
				message.content[index] = { type: "text", text: "" };
				break;
			case "text_delta":
				if (!message.content[index]) message.content[index] = { type: "text", text: "" };
				message.content[index].text = `${message.content[index].text || ""}${delta.delta || ""}`;
				break;
			case "text_end":
				message.content[index] = { type: "text", text: delta.content || message.content[index]?.text || "" };
				break;
			case "thinking_start":
				message.content[index] = { type: "thinking", thinking: "" };
				break;
			case "thinking_delta":
				if (!message.content[index]) message.content[index] = { type: "thinking", thinking: "" };
				message.content[index].thinking = `${message.content[index].thinking || ""}${delta.delta || ""}`;
				break;
			case "thinking_end":
				message.content[index] = {
					type: "thinking",
					thinking: delta.content || message.content[index]?.thinking || "",
				};
				break;
			case "toolcall_end":
				message.content[index] = delta.toolCall;
				break;
		}
		renderMessages(true);
	}

	function handleMessageEnd(message) {
		if (!message) return;
		if (message.role === "assistant" && state.streamingIndex >= 0) {
			state.items[state.streamingIndex] = { kind: "message", message };
			state.streamingIndex = -1;
		}
		renderMessages(true);
	}

	function startTool(event) {
		const item = {
			kind: "tool",
			id: event.toolCallId,
			name: event.toolName || "tool",
			args: event.args || {},
			result: null,
			complete: false,
			error: false,
		};
		state.tools.set(item.id, item);
		state.items.push(item);
		renderMessages(true);
	}

	function updateTool(event, complete) {
		let item = state.tools.get(event.toolCallId);
		if (!item) {
			item = { kind: "tool", id: event.toolCallId, name: event.toolName || "tool", args: event.args || {} };
			state.tools.set(item.id, item);
			state.items.push(item);
		}
		item.result = complete ? event.result : event.partialResult;
		item.complete = complete;
		item.error = Boolean(event.isError);
		renderMessages(true);
	}

	function renderSessions() {
		elements.sessionList.replaceChildren();
		for (const session of state.sessions) {
			const button = document.createElement("button");
			button.type = "button";
			button.className = `session-row${session.catalog_id === state.active?.catalog_id ? " active" : ""}`;
			button.innerHTML = `
				<span class="session-row-main">
					<span class="session-row-title">${session.running ? '<i class="running-dot"></i>' : ""}${escapeHtml(session.title)}</span>
					<span class="session-row-meta">${escapeHtml(shortPath(session.cwd))}</span>
				</span>
				<time class="session-row-time">${formatRelativeTime(session.modified_at)}</time>`;
			button.addEventListener("click", () => requestSession(session));
			elements.sessionList.append(button);
		}
	}

	function renderMessages(stickToBottom) {
		const wasNearBottom =
			elements.conversation.scrollHeight - elements.conversation.scrollTop - elements.conversation.clientHeight < 120;
		elements.messageList.replaceChildren();
		let userIndex = 0;
		for (const item of state.items) {
			const node = item.kind === "tool" ? renderTool(item) : renderMessage(item.message, userIndex);
			if (!node) continue;
			if (item.kind === "message" && item.message?.role === "user") userIndex += 1;
			elements.messageList.append(node);
		}
		elements.empty.classList.toggle("hidden", state.items.length > 0 || Boolean(state.active));
		if (stickToBottom && wasNearBottom) requestAnimationFrame(scrollToBottom);
	}

	function renderMessage(message, userIndex) {
		if (!message) return null;
		if (message.role === "user") {
			const node = document.createElement("div");
			node.className = "message user";
			node.dataset.userIndex = String(userIndex);
			node.textContent = textContent(message.content);
			return node;
		}
		if (message.role === "assistant") {
			const node = document.createElement("div");
			node.className = `message assistant${state.streaming && message === state.items[state.streamingIndex]?.message ? " stream-cursor" : ""}`;
			for (const block of Array.isArray(message.content) ? message.content : []) {
				if (!block) continue;
				if (block.type === "text" && block.text) {
					const part = document.createElement("div");
					part.innerHTML = renderMarkdown(block.text);
					node.append(part);
				}
				if (block.type === "thinking" && block.thinking) node.append(renderThinking(block.thinking));
			}
			if (message.errorMessage) {
				const error = document.createElement("p");
				error.textContent = message.errorMessage;
				node.append(error);
			}
			return node;
		}
		if (message.role === "toolResult") {
			return renderTool({
				kind: "tool",
				id: message.toolCallId,
				name: message.toolName,
				result: message,
				complete: true,
				error: message.isError,
			});
		}
		return null;
	}

	function renderThinking(text) {
		const details = document.createElement("details");
		details.className = "thinking";
		const summary = document.createElement("summary");
		summary.textContent = "思考";
		const body = document.createElement("div");
		body.className = "thinking-body";
		body.textContent = text;
		details.append(summary, body);
		return details;
	}

	function renderTool(item) {
		const details = document.createElement("details");
		details.className = `tool-card${item.error ? " error" : ""}`;
		const summary = document.createElement("summary");
		summary.className = "tool-head";
		const name = document.createElement("span");
		name.textContent = toolLabel(item);
		const status = document.createElement("span");
		status.className = "tool-state";
		status.textContent = item.complete ? (item.error ? "失败" : "完成") : "运行中";
		summary.append(name, status);
		const body = document.createElement("div");
		body.className = "tool-body";
		const output = resultText(item.result);
		body.textContent = output || JSON.stringify(item.args || {}, null, 2);
		details.append(summary, body);
		return details;
	}

	function renderTimeline() {
		elements.timelineList.replaceChildren();
		const entries = state.entries.filter(
			(entry) => entry?.type === "message" && entry.message?.role === "user",
		);
		entries.forEach((entry, index) => {
			const button = document.createElement("button");
			button.type = "button";
			button.className = "timeline-row";
			const timestamp = entry.timestamp || entry.message?.timestamp;
			button.innerHTML = `<time>${formatClock(timestamp)}</time><span class="timeline-text">${escapeHtml(textContent(entry.message.content))}</span>`;
			button.addEventListener("click", () => jumpToUserMessage(index));
			elements.timelineList.append(button);
		});
	}

	function jumpToUserMessage(index) {
		closeTimeline();
		requestAnimationFrame(() => {
			const target = elements.messageList.querySelector(`[data-user-index="${index}"]`);
			target?.scrollIntoView({ behavior: "smooth", block: "center" });
		});
	}

	function submitPrompt() {
		if (state.streaming) {
			send({ type: "abort", runtime_id: state.active?.runtime_id });
			return;
		}
		const message = elements.prompt.value.trim();
		if (!message || !state.active?.runtime_id) return;
		if (
			send({
				type: "prompt",
				runtime_id: state.active.runtime_id,
				message,
				behavior: "normal",
			})
		) {
			elements.prompt.value = "";
			resizePrompt();
		}
	}

	function openMenu() {
		elements.app.classList.add("menu-open");
		elements.menuToggle.setAttribute("aria-expanded", "true");
		elements.menuToggle.setAttribute("aria-label", "关闭会话菜单");
		elements.sessionMenu.setAttribute("aria-hidden", "false");
		send({ type: "list_sessions" });
	}

	function closeMenu() {
		elements.app.classList.remove("menu-open");
		elements.menuToggle.setAttribute("aria-expanded", "false");
		elements.menuToggle.setAttribute("aria-label", "打开会话菜单");
		elements.sessionMenu.setAttribute("aria-hidden", "true");
	}

	function openTimeline() {
		if (!state.active) return;
		elements.app.classList.add("timeline-open");
		elements.timeline.setAttribute("aria-hidden", "false");
	}

	function closeTimeline() {
		elements.app.classList.remove("timeline-open");
		elements.timeline.setAttribute("aria-hidden", "true");
	}

	function installGesture(target) {
		let gesture = null;
		target.addEventListener(
			"touchstart",
			(event) => {
				if (event.touches.length !== 2) {
					gesture = null;
					return;
				}
				const [first, second] = event.touches;
				gesture = {
					startX: (first.clientX + second.clientX) / 2,
					startY: (first.clientY + second.clientY) / 2,
					lastX: (first.clientX + second.clientX) / 2,
					lastY: (first.clientY + second.clientY) / 2,
					distance: Math.hypot(first.clientX - second.clientX, first.clientY - second.clientY),
					claimed: false,
				};
			},
			{ passive: true },
		);
		target.addEventListener(
			"touchmove",
			(event) => {
				if (!gesture || event.touches.length !== 2) return;
				const [first, second] = event.touches;
				gesture.lastX = (first.clientX + second.clientX) / 2;
				gesture.lastY = (first.clientY + second.clientY) / 2;
				const currentDistance = Math.hypot(first.clientX - second.clientX, first.clientY - second.clientY);
				const distanceChange = Math.abs(currentDistance - gesture.distance) / Math.max(gesture.distance, 1);
				const dx = gesture.lastX - gesture.startX;
				const dy = gesture.lastY - gesture.startY;
				if (Math.abs(dy) > 22 && Math.abs(dy) > Math.abs(dx) * 1.4 && distanceChange < 0.15) {
					gesture.claimed = true;
					event.preventDefault();
				}
			},
			{ passive: false },
		);
		target.addEventListener("touchend", () => {
			if (!gesture?.claimed) {
				gesture = null;
				return;
			}
			const dy = gesture.lastY - gesture.startY;
			if (dy > 58) openTimeline();
			if (dy < -58) closeTimeline();
			gesture = null;
		});
		target.addEventListener("touchcancel", () => {
			gesture = null;
		});
	}

	function updateViewport() {
		const viewport = window.visualViewport;
		const height = viewport ? viewport.height : window.innerHeight;
		document.documentElement.style.setProperty("--app-height", `${Math.round(height)}px`);
	}

	function updateSendButton() {
		elements.send.classList.toggle("streaming", state.streaming);
		elements.send.setAttribute("aria-label", state.streaming ? "停止生成" : "发送消息");
		elements.send.disabled = !state.streaming && (!state.active || !elements.prompt.value.trim());
	}

	function resizePrompt() {
		elements.prompt.style.height = "auto";
		elements.prompt.style.height = `${Math.min(elements.prompt.scrollHeight, 132)}px`;
		updateSendButton();
	}

	function scrollToBottom() {
		elements.conversation.scrollTop = elements.conversation.scrollHeight;
	}

	function setEmptyStatus(text) {
		const status = elements.empty.querySelector("small");
		if (status) status.textContent = text;
	}

	function showToast(message) {
		window.clearTimeout(state.toastTimer);
		elements.toast.textContent = message;
		elements.toast.classList.add("visible");
		state.toastTimer = window.setTimeout(() => elements.toast.classList.remove("visible"), 3200);
	}

	function textContent(content) {
		if (typeof content === "string") return content;
		if (!Array.isArray(content)) return "";
		return content
			.filter((block) => block?.type === "text")
			.map((block) => block.text || "")
			.join("\n");
	}

	function resultText(result) {
		const content = result?.content;
		if (!Array.isArray(content)) return "";
		return content
			.filter((block) => block?.type === "text")
			.map((block) => block.text || "")
			.join("\n");
	}

	function toolLabel(item) {
		const path = item.args?.path || item.args?.file_path || item.args?.filePath;
		const command = item.args?.command;
		const detail = path || command;
		return detail ? `${item.name} · ${String(detail).slice(0, 72)}` : item.name;
	}

	function renderMarkdown(source) {
		const escaped = escapeHtml(source);
		const blocks = escaped.split(/(```[\s\S]*?```)/g);
		return blocks
			.map((block) => {
				if (block.startsWith("```")) {
					const body = block.replace(/^```[^\n]*\n?/, "").replace(/```$/, "");
					return `<pre><code>${body}</code></pre>`;
				}
				return block
					.split(/\n{2,}/)
					.map((paragraph) => `<p>${paragraph.replace(/\n/g, "<br>").replace(/`([^`]+)`/g, "<code>$1</code>")}</p>`)
					.join("");
			})
			.join("");
	}

	function escapeHtml(value) {
		return String(value ?? "")
			.replaceAll("&", "&amp;")
			.replaceAll("<", "&lt;")
			.replaceAll(">", "&gt;")
			.replaceAll('"', "&quot;")
			.replaceAll("'", "&#039;");
	}

	function shortPath(value) {
		const parts = String(value || "").split(/[\\/]/).filter(Boolean);
		return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : value || "";
	}

	function formatRelativeTime(value) {
		const time = new Date(value).getTime();
		if (!Number.isFinite(time)) return "";
		const seconds = Math.max(0, Math.round((Date.now() - time) / 1000));
		if (seconds < 60) return "刚刚";
		if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟`;
		if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时`;
		if (seconds < 604800) return `${Math.floor(seconds / 86400)} 天`;
		return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" }).format(new Date(time));
	}

	function formatClock(value) {
		const time = typeof value === "number" ? new Date(value) : new Date(value || Date.now());
		return new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }).format(time);
	}

	elements.menuToggle.addEventListener("click", () => {
		if (elements.app.classList.contains("menu-open")) closeMenu();
		else openMenu();
	});
	elements.title.addEventListener("click", openTimeline);
	elements.newSession.addEventListener("click", () => {
		closeMenu();
		send({ type: "create_session" });
	});
	elements.send.addEventListener("click", submitPrompt);
	elements.prompt.addEventListener("input", resizePrompt);
	elements.prompt.addEventListener("keydown", (event) => {
		if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
			event.preventDefault();
			submitPrompt();
		}
	});
	document.addEventListener("visibilitychange", () => {
		if (document.visibilityState === "visible" && (!state.ws || state.ws.readyState > WebSocket.OPEN)) connect();
	});
	window.visualViewport?.addEventListener("resize", updateViewport);
	window.visualViewport?.addEventListener("scroll", updateViewport);
	window.addEventListener("resize", updateViewport);
	installGesture(elements.conversation);
	installGesture(elements.timeline);
	updateViewport();
	resizePrompt();
	connect();
})();
