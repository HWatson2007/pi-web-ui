#!/usr/bin/env node

import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const port = 39123;
const temp = await fs.mkdtemp(path.join(os.tmpdir(), "pi-mobile-ui-"));
const server = spawn(path.join(project, "target/release/pi-mobile-ui"), [
	"--host",
	"127.0.0.1",
	"--port",
	String(port),
	"--cwd",
	project,
	"--agent-dir",
	path.join(temp, "agent"),
	"--pi-binary",
	path.join(project, "tests/fixtures/fake-pi.mjs"),
], { stdio: ["ignore", "pipe", "pipe"] });

let logs = "";
server.stdout.on("data", (chunk) => (logs += chunk));
server.stderr.on("data", (chunk) => (logs += chunk));

async function waitForServer() {
	for (let attempt = 0; attempt < 80; attempt += 1) {
		try {
			const response = await fetch(`http://127.0.0.1:${port}/api/health`);
			if (response.ok) return;
		} catch {}
		await new Promise((resolve) => setTimeout(resolve, 50));
	}
	throw new Error(`server did not start\n${logs}`);
}

async function nextMessage(inbox, predicate, timeoutMs = 5_000) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const index = inbox.findIndex(predicate);
		if (index >= 0) return inbox.splice(index, 1)[0];
		await new Promise((resolve) => setTimeout(resolve, 10));
	}
	throw new Error(`timed out waiting for WebSocket message\n${logs}\n${JSON.stringify(inbox)}`);
}

try {
	await waitForServer();
	const index = await (await fetch(`http://127.0.0.1:${port}/`)).text();
	const css = await (await fetch(`http://127.0.0.1:${port}/styles.css`)).text();
	assert.match(index, /id="session-menu"/);
	assert.match(css, /visualViewport|composer|border-top/);

	const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
	const inbox = [];
	ws.addEventListener("message", (event) => inbox.push(JSON.parse(event.data)));
	await new Promise((resolve, reject) => {
		ws.addEventListener("open", resolve, { once: true });
		ws.addEventListener("error", reject, { once: true });
	});
	ws.send(JSON.stringify({ type: "create_session" }));
	const opened = await nextMessage(inbox, (message) => message.type === "session_opened");
	assert.equal(opened.session.running, true);
	assert.ok(opened.session.runtime_id);

	ws.send(JSON.stringify({
		type: "prompt",
		runtime_id: opened.session.runtime_id,
		message: "测试手机端",
		behavior: "normal",
	}));
	const delta = await nextMessage(
		inbox,
		(message) =>
			message.type === "rpc_event" &&
			message.event?.assistantMessageEvent?.type === "text_delta",
	);
	assert.equal(delta.event.assistantMessageEvent.delta, "收到，RPC 正常。");
	await nextMessage(
		inbox,
		(message) => message.type === "rpc_event" && message.event?.type === "agent_settled",
	);
	ws.close();
	console.log("smoke test passed: HTTP assets, WebSocket, session creation, prompt streaming");
} finally {
	server.kill("SIGTERM");
	await new Promise((resolve) => server.once("exit", resolve));
	await fs.rm(temp, { recursive: true, force: true });
}
