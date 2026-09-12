#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const args = process.argv.slice(2);
const sessionIndex = args.indexOf("--session");
const agentDir = process.env.PI_CODING_AGENT_DIR || path.join(process.cwd(), ".pi-agent");
const sessionFile =
	sessionIndex >= 0
		? path.resolve(args[sessionIndex + 1])
		: path.join(agentDir, "sessions", "fake", `session-${process.pid}.jsonl`);
const sessionId = path.basename(sessionFile, ".jsonl");
let sessionName = "新对话";
const messages = [];
const entries = [];

fs.mkdirSync(path.dirname(sessionFile), { recursive: true });
if (!fs.existsSync(sessionFile)) {
	fs.writeFileSync(
		sessionFile,
		`${JSON.stringify({ type: "session", version: 3, id: sessionId, timestamp: new Date().toISOString(), cwd: process.cwd() })}\n`,
	);
}

function emit(value) {
	process.stdout.write(`${JSON.stringify(value)}\n`);
}

function respond(request, data = null) {
	emit({ type: "response", command: request.type, success: true, id: request.id, data });
}

function handle(request) {
	switch (request.type) {
		case "get_state":
			respond(request, {
				model: { provider: "fake", id: "fake-model" },
				thinkingLevel: "medium",
				isStreaming: false,
				sessionFile,
				sessionId,
				sessionName,
				messageCount: messages.length,
			});
			break;
		case "get_messages":
			respond(request, { messages });
			break;
		case "get_entries":
			respond(request, { entries, leafId: entries.at(-1)?.id || null });
			break;
		case "set_session_name":
			sessionName = request.name;
			respond(request);
			break;
		case "prompt": {
			respond(request);
			const now = Date.now();
			const user = { role: "user", content: [{ type: "text", text: request.message }], timestamp: now };
			const assistant = {
				role: "assistant",
				content: [{ type: "text", text: "收到，RPC 正常。" }],
				timestamp: now + 1,
			};
			messages.push(user, assistant);
			entries.push({ type: "message", id: `u-${now}`, timestamp: new Date(now).toISOString(), message: user });
			setTimeout(() => {
				emit({ type: "agent_start" });
				emit({ type: "message_start", message: user });
				emit({ type: "message_end", message: user });
				emit({ type: "message_start", message: { role: "assistant", content: [], timestamp: now + 1 } });
				emit({ type: "message_update", assistantMessageEvent: { type: "text_start", contentIndex: 0 } });
				emit({
					type: "message_update",
					assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: "收到，RPC 正常。" },
				});
				emit({
					type: "message_update",
					assistantMessageEvent: { type: "text_end", contentIndex: 0, content: "收到，RPC 正常。" },
				});
				emit({ type: "message_end", message: assistant });
				emit({ type: "agent_end", messages: [assistant], willRetry: false });
				emit({ type: "agent_settled" });
			}, 10);
			break;
		}
		default:
			respond(request);
	}
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
	buffer += chunk;
	for (;;) {
		const newline = buffer.indexOf("\n");
		if (newline < 0) break;
		const record = buffer.slice(0, newline).replace(/\r$/, "");
		buffer = buffer.slice(newline + 1);
		if (record) handle(JSON.parse(record));
	}
});

