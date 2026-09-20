import path from "node:path";
import * as vscode from "vscode";
import type { LintDiagnostic, Snapshot } from "./protocol.js";

type FileDiagnostics = {
	uri: vscode.Uri;
	diagnostics: readonly vscode.Diagnostic[];
};

export class DiagnosticStore implements vscode.Disposable {
	private readonly collection =
		vscode.languages.createDiagnosticCollection("jevlint");
	private readonly byWatcher = new Map<string, Map<string, FileDiagnostics>>();

	update(watcherId: string, snapshot: Snapshot): void {
		const next = groupDiagnostics(snapshot);
		const previous = this.byWatcher.get(watcherId) ?? new Map();
		this.byWatcher.set(watcherId, next);
		this.refresh(new Set([...previous.keys(), ...next.keys()]));
	}

	remove(watcherId: string): void {
		const previous = this.byWatcher.get(watcherId);
		if (previous === undefined) return;
		this.byWatcher.delete(watcherId);
		this.refresh(new Set(previous.keys()));
	}

	dispose(): void {
		this.byWatcher.clear();
		this.collection.dispose();
	}

	private refresh(paths: ReadonlySet<string>): void {
		for (const filePath of paths) {
			let uri: vscode.Uri | undefined;
			const diagnostics: vscode.Diagnostic[] = [];
			for (const files of this.byWatcher.values()) {
				const file = files.get(filePath);
				if (file !== undefined) {
					uri = file.uri;
					diagnostics.push(...file.diagnostics);
				}
			}
			if (uri === undefined || diagnostics.length === 0)
				this.collection.delete(vscode.Uri.file(filePath));
			else this.collection.set(uri, diagnostics);
		}
	}
}

function groupDiagnostics(snapshot: Snapshot): Map<string, FileDiagnostics> {
	const grouped = new Map<string, FileDiagnostics>();
	for (const lint of snapshot.diagnostics) {
		const filePath = path.resolve(snapshot.root, lint.path);
		const uri = vscode.Uri.file(filePath);
		const existing = grouped.get(filePath)?.diagnostics ?? [];
		grouped.set(filePath, {
			uri,
			diagnostics: [...existing, ...toDiagnostics(lint)],
		});
	}
	return grouped;
}

function toDiagnostics(lint: LintDiagnostic): vscode.Diagnostic[] {
	const regions =
		lint.regions.length === 0 ? [{ startLine: 1, endLine: 1 }] : lint.regions;
	return regions.map((region) => {
		const range = new vscode.Range(
			region.startLine - 1,
			0,
			region.endLine - 1,
			Number.MAX_SAFE_INTEGER,
		);
		const severity =
			lint.severity === "error"
				? vscode.DiagnosticSeverity.Error
				: vscode.DiagnosticSeverity.Warning;
		const confidence = `${Math.round(lint.confidence * 100)}% confidence`;
		const diagnostic = new vscode.Diagnostic(
			range,
			`${lint.ruleId} (${confidence})`,
			severity,
		);
		diagnostic.source = "jevlint";
		diagnostic.code = lint.ruleId;
		return diagnostic;
	});
}
