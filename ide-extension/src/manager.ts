import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import * as vscode from "vscode";
import { DiagnosticStore } from "./diagnostics.js";
import type { Snapshot, WatchMessage } from "./protocol.js";
import { type TraceLevel, WatchSession } from "./watcher.js";

type WatcherState =
	| { kind: "running" }
	| { kind: "snapshot"; snapshot: Snapshot }
	| { kind: "unavailable"; message: string };

export class JevLintManager implements vscode.Disposable {
	private readonly output = vscode.window.createOutputChannel("JevLint");
	private readonly status = vscode.window.createStatusBarItem(
		vscode.StatusBarAlignment.Left,
		100,
	);
	private readonly diagnostics = new DiagnosticStore();
	private readonly sessions = new Map<string, WatchSession>();
	private readonly states = new Map<string, WatcherState>();
	private readonly subscriptions: vscode.Disposable[] = [];
	private refreshTimer: NodeJS.Timeout | undefined;
	private refreshGeneration = 0;
	private disposed = false;

	constructor(context: vscode.ExtensionContext) {
		this.status.command = "jevlint.showOutput";
		this.status.name = "JevLint";
		this.subscriptions.push(
			vscode.commands.registerCommand("jevlint.restart", () => this.restart()),
			vscode.commands.registerCommand("jevlint.showOutput", () =>
				this.output.show(),
			),
			vscode.workspace.onDidChangeWorkspaceFolders(() =>
				this.scheduleRefresh(),
			),
			vscode.workspace.onDidChangeConfiguration((event) => {
				if (event.affectsConfiguration("jevlint")) void this.restart();
			}),
		);

		const configs = vscode.workspace.createFileSystemWatcher(
			"**/.jevlintrc.jsonc",
		);
		this.subscriptions.push(
			configs,
			configs.onDidCreate(() => this.scheduleRefresh()),
			configs.onDidDelete(() => this.scheduleRefresh()),
			configs.onDidChange(() => this.scheduleRefresh()),
		);
		if (!vscode.workspace.isTrusted) {
			this.status.text = "$(lock) JevLint";
			this.status.tooltip =
				"JevLint is disabled until this workspace is trusted.";
			this.status.show();
			this.subscriptions.push(
				vscode.workspace.onDidGrantWorkspaceTrust(() => this.refresh()),
			);
		}
		context.subscriptions.push(this);
	}

	async start(): Promise<void> {
		if (vscode.workspace.isTrusted) await this.refresh();
	}

	dispose(): void {
		this.disposed = true;
		this.refreshGeneration += 1;
		if (this.refreshTimer !== undefined) clearTimeout(this.refreshTimer);
		for (const session of this.sessions.values()) session.dispose();
		this.sessions.clear();
		this.states.clear();
		for (const subscription of this.subscriptions) subscription.dispose();
		this.diagnostics.dispose();
		this.status.dispose();
		this.output.dispose();
	}

	private async restart(): Promise<void> {
		this.stopAll();
		await this.refresh();
	}

	private stopAll(): void {
		for (const [id, session] of this.sessions) {
			session.dispose();
			this.diagnostics.remove(id);
		}
		this.sessions.clear();
		this.states.clear();
		this.refreshStatus();
	}

	private scheduleRefresh(): void {
		if (this.refreshTimer !== undefined) clearTimeout(this.refreshTimer);
		this.refreshTimer = setTimeout(() => {
			this.refreshTimer = undefined;
			void this.refresh();
		}, 200);
	}

	private async refresh(): Promise<void> {
		if (this.disposed || !vscode.workspace.isTrusted) return;
		const generation = ++this.refreshGeneration;
		const paths = await discoverConfigs();
		if (this.disposed || generation !== this.refreshGeneration) return;

		const desired = new Set(paths);
		for (const [id, session] of this.sessions) {
			if (!desired.has(id)) {
				session.dispose();
				this.sessions.delete(id);
				this.states.delete(id);
				this.diagnostics.remove(id);
			}
		}
		const executable = resolveExecutable();
		for (const configPath of paths) {
			if (this.sessions.has(configPath)) continue;
			const session = new WatchSession(configPath, executable, {
				onMessage: (id, message) => this.handleMessage(id, message),
				onUnavailable: (id, message) => this.handleUnavailable(id, message),
				log: (message) => this.output.appendLine(message),
				traceLevel: () => traceLevel(),
			});
			this.sessions.set(configPath, session);
			this.states.set(configPath, { kind: "running" });
			session.start();
		}
		if (paths.length === 0)
			this.output.appendLine("No .jevlintrc.jsonc files found.");
		this.refreshStatus();
	}

	private handleMessage(id: string, message: WatchMessage): void {
		if (!this.sessions.has(id)) return;
		if (message.kind === "run_started") {
			this.states.set(id, { kind: "running" });
		} else {
			this.states.set(id, { kind: "snapshot", snapshot: message });
			this.diagnostics.update(id, message);
			if (message.status === "error" && message.error !== null) {
				this.output.appendLine(`[${id}] ${message.error}`);
			}
			if (message.precondition !== null) {
				this.output.appendLine(
					`[${id}] precondition failed: ${message.precondition.command.join(" ")}`,
				);
				if (message.precondition.output.length > 0) {
					this.output.appendLine(message.precondition.output);
				}
			}
		}
		this.refreshStatus();
	}

	private handleUnavailable(id: string, message: string): void {
		if (!this.sessions.has(id)) return;
		this.states.set(id, { kind: "unavailable", message });
		this.diagnostics.remove(id);
		this.refreshStatus();
	}

	private refreshStatus(): void {
		if (this.sessions.size === 0) {
			this.status.hide();
			return;
		}
		const states = [...this.states.values()];
		const unavailable = states.find((state) => state.kind === "unavailable");
		if (unavailable?.kind === "unavailable") {
			this.status.text = "$(error) JevLint unavailable";
			this.status.tooltip = unavailable.message;
		} else if (states.some((state) => state.kind === "running")) {
			this.status.text = "$(sync~spin) JevLint";
			this.status.tooltip = "JevLint is checking the workspace.";
		} else {
			const snapshots = states.flatMap((state) =>
				state.kind === "snapshot" ? [state.snapshot] : [],
			);
			const failed = snapshots.find(
				(snapshot) =>
					snapshot.status === "error" ||
					snapshot.status === "precondition_failed",
			);
			const errors = countSeverity(snapshots, "error");
			const warnings = countSeverity(snapshots, "warning");
			if (failed !== undefined) {
				this.status.text = "$(error) JevLint blocked";
				this.status.tooltip =
					failed.error ??
					"A configured precondition failed. See JevLint output.";
			} else if (errors > 0) {
				this.status.text = `$(error) JevLint ${errors}`;
				this.status.tooltip = `${errors} errors and ${warnings} warnings`;
			} else if (warnings > 0) {
				this.status.text = `$(warning) JevLint ${warnings}`;
				this.status.tooltip = `${warnings} warnings`;
			} else {
				this.status.text = "$(check) JevLint";
				this.status.tooltip = "No JevLint violations.";
			}
		}
		this.status.show();
	}
}

function configuration(): vscode.WorkspaceConfiguration {
	return vscode.workspace.getConfiguration("jevlint");
}

function traceLevel(): TraceLevel {
	const value = configuration().get<string>("trace.server", "off");
	return value === "messages" || value === "verbose" ? value : "off";
}

function resolveExecutable(): string {
	const configured = configuration().get<string>("executablePath", "jevlint");
	if (configured !== "jevlint") return configured;
	const localInstall = path.join(os.homedir(), ".local", "bin", "jevlint");
	return existsSync(localInstall) ? localInstall : configured;
}

async function discoverConfigs(): Promise<string[]> {
	const folders = vscode.workspace.workspaceFolders ?? [];
	const found = await Promise.all(
		folders.map((folder) =>
			vscode.workspace.findFiles(
				new vscode.RelativePattern(folder, "**/.jevlintrc.jsonc"),
				"**/{.git,.jevlint,node_modules,target,.venv,venv}/**",
			),
		),
	);
	return [...new Set(found.flat().map((uri) => uri.fsPath))].sort();
}

function countSeverity(
	snapshots: readonly Snapshot[],
	severity: "error" | "warning",
): number {
	return snapshots.reduce(
		(total, snapshot) =>
			total +
			snapshot.diagnostics.filter(
				(diagnostic) => diagnostic.severity === severity,
			).length,
		0,
	);
}
