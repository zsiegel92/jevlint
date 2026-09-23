import { spawn, type ChildProcessByStdio } from "node:child_process";
import path from "node:path";
import { createInterface, type Interface } from "node:readline";
import type { Readable } from "node:stream";
import type { Snapshot, WatchMessage } from "./protocol.js";
import { parseWatchMessage } from "./protocol.js";

export type TraceLevel = "off" | "messages" | "verbose";

export type WatcherCallbacks = {
	onMessage(id: string, message: WatchMessage): void;
	onUnavailable(id: string, message: string): void;
	log(message: string): void;
	traceLevel(): TraceLevel;
};

export class WatchSession {
	readonly id: string;
	readonly configPath: string;
	private readonly executable: string;
	private readonly callbacks: WatcherCallbacks;
	private child: ChildProcessByStdio<null, Readable, Readable> | undefined;
	private readers: Interface[] = [];
	private restartTimer: NodeJS.Timeout | undefined;
	private restartDelay = 1_000;
	private generation = 0;
	private disposed = false;

	constructor(
		configPath: string,
		executable: string,
		callbacks: WatcherCallbacks,
	) {
		this.id = configPath;
		this.configPath = configPath;
		this.executable = executable;
		this.callbacks = callbacks;
	}

	start(): void {
		this.launch();
	}

	dispose(): void {
		this.disposed = true;
		this.generation += 1;
		if (this.restartTimer !== undefined) clearTimeout(this.restartTimer);
		this.restartTimer = undefined;
		for (const reader of this.readers) reader.close();
		this.readers = [];
		this.child?.removeAllListeners();
		this.child?.kill();
		this.child = undefined;
	}

	private launch(): void {
		if (this.disposed) return;
		const generation = ++this.generation;
		this.callbacks.log(
			`[${this.configPath}] starting ${this.executable} watch`,
		);
		const child = spawn(
			this.executable,
			["watch", "--stream", "--config", this.configPath],
			{
				cwd: path.dirname(this.configPath),
				env: process.env,
				stdio: ["ignore", "pipe", "pipe"],
				windowsHide: true,
			},
		);
		this.child = child;
		const stdout = createInterface({ input: child.stdout });
		const stderr = createInterface({ input: child.stderr });
		this.readers = [stdout, stderr];
		stdout.on("line", (line) => this.handleLine(line));
		stderr.on("line", (line) =>
			this.callbacks.log(`[${this.configPath}] ${line}`),
		);
		child.on("error", (error) => {
			this.callbacks.log(
				`[${this.configPath}] process error: ${error.message}`,
			);
		});
		child.on("close", (code, signal) => {
			if (generation !== this.generation || this.disposed) return;
			this.child = undefined;
			stdout.close();
			stderr.close();
			const detail = `watch process exited (${signal ?? code ?? "unknown"})`;
			this.callbacks.log(`[${this.configPath}] ${detail}`);
			this.callbacks.onUnavailable(this.id, detail);
			this.scheduleRestart(generation);
		});
	}

	private handleLine(line: string): void {
		if (this.callbacks.traceLevel() === "verbose") {
			this.callbacks.log(`[${this.configPath}] <= ${line}`);
		}
		try {
			const message = parseWatchMessage(line);
			if (message.kind === "snapshot") this.acceptSnapshot(message);
			this.callbacks.onMessage(this.id, message);
		} catch (error) {
			const detail = error instanceof Error ? error.message : String(error);
			this.callbacks.log(
				`[${this.configPath}] invalid protocol message: ${detail}`,
			);
		}
	}

	private acceptSnapshot(snapshot: Snapshot): void {
		this.restartDelay = 1_000;
		if (this.callbacks.traceLevel() === "messages") {
			this.callbacks.log(
				`[${this.configPath}] snapshot ${snapshot.sequence}: ${snapshot.status}, ${snapshot.diagnostics.length} diagnostics`,
			);
		}
	}

	private scheduleRestart(generation: number): void {
		const delay = this.restartDelay;
		this.restartDelay = Math.min(this.restartDelay * 2, 30_000);
		this.restartTimer = setTimeout(() => {
			if (generation === this.generation) this.launch();
		}, delay);
	}
}
