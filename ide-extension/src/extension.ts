import type * as vscode from "vscode";
import { JevLintManager } from "./manager.js";

export async function activate(
	context: vscode.ExtensionContext,
): Promise<void> {
	const manager = new JevLintManager(context);
	await manager.start();
}

export function deactivate(): void {}
