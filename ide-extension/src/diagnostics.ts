import path from "node:path";
import * as vscode from "vscode";
import type { FileResult, LintDiagnostic, Snapshot } from "./protocol.js";

type FileDiagnostics = {
	uri: vscode.Uri;
	diagnostics: readonly vscode.Diagnostic[];
};

export class DiagnosticStore implements vscode.Disposable {
	private readonly collection =
		vscode.languages.createDiagnosticCollection("jevlint");
	private readonly byWatcher = new Map<string, Map<string, FileDiagnostics>>();

	update(watcherId: string, snapshot: Snapshot): void {
		const incoming = groupDiagnostics(snapshot.root, snapshot.diagnostics);
		const previous = this.byWatcher.get(watcherId) ?? new Map();
		const current = snapshot.fullUpdate ? new Map(incoming) : new Map(previous);
		const updated = snapshot.fullUpdate
			? new Set([...previous.keys(), ...incoming.keys()])
			: new Set(
					snapshot.updatedPaths.map((filePath) =>
						path.resolve(snapshot.root, filePath),
					),
				);
		if (!snapshot.fullUpdate) {
			for (const filePath of updated) {
				const replacement = incoming.get(filePath);
				if (replacement === undefined) current.delete(filePath);
				else current.set(filePath, replacement);
			}
		}
		this.byWatcher.set(watcherId, current);
		this.refresh(updated);
	}

	updateFile(watcherId: string, result: FileResult): void {
		const filePath = path.resolve(result.root, result.path);
		const current =
			this.byWatcher.get(watcherId) ?? new Map<string, FileDiagnostics>();
		const replacement = groupDiagnostics(result.root, result.diagnostics).get(
			filePath,
		);
		if (replacement === undefined) current.delete(filePath);
		else current.set(filePath, replacement);
		this.byWatcher.set(watcherId, current);
		this.refresh(new Set([filePath]));
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

function groupDiagnostics(
	root: string,
	diagnostics: readonly LintDiagnostic[],
): Map<string, FileDiagnostics> {
	const grouped = new Map<string, FileDiagnostics>();
	for (const lint of diagnostics) {
		const filePath = path.resolve(root, lint.path);
		const uri = vscode.Uri.file(filePath);
		const existing = grouped.get(filePath)?.diagnostics ?? [];
		grouped.set(filePath, {
			uri,
			diagnostics: [...existing, ...toDiagnostics(root, lint)],
		});
	}
	return grouped;
}

function toDiagnostics(
	root: string,
	lint: LintDiagnostic,
): vscode.Diagnostic[] {
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
			`${plainMessage(lint.message)} (${confidence})`,
			severity,
		);
		diagnostic.source = "jevlint";
		diagnostic.code = {
			value: lint.ruleId,
			target: vscode.Uri.file(path.resolve(root, lint.rulePath)),
		};
		return diagnostic;
	});
}

function plainMessage(markdown: string): string {
	return markdown
		.split("\n")
		.map((line) => line.trim().replace(/^#+\s*/, ""))
		.filter((line) => line.length > 0)
		.join(" ");
}
