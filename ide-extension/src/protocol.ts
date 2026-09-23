export type Severity = "error" | "warning";

export type RunStarted = {
	schemaVersion: 1;
	kind: "run_started";
	sequence: number;
	trigger: "initial" | "filesystem";
	changedPaths: readonly string[];
};

export type Region = {
	startLine: number;
	endLine: number;
};

export type LintDiagnostic = {
	path: string;
	ruleId: string;
	rulePath: string;
	message: string;
	severity: Severity;
	confidence: number;
	regions: readonly Region[];
};

export type SnapshotStatus =
	| "clean"
	| "violations"
	| "precondition_failed"
	| "error";

export type SnapshotStats = {
	filesChecked: number;
	rules: number;
	apiRequests: number;
	cacheHits: number;
	violations: number;
	errors: number;
	warnings: number;
};

export type PreconditionStatus = {
	command: readonly string[];
	exitCode: number | null;
	output: string;
	skippedFiles: readonly string[];
};

export type Snapshot = {
	schemaVersion: 1;
	kind: "snapshot";
	sequence: number;
	root: string;
	lineBase: 1;
	status: SnapshotStatus;
	fullUpdate: boolean;
	updatedPaths: readonly string[];
	diagnostics: readonly LintDiagnostic[];
	stats: SnapshotStats | null;
	precondition: PreconditionStatus | null;
	error: string | null;
};

export type FileResult = {
	schemaVersion: 1;
	kind: "file_result";
	sequence: number;
	root: string;
	path: string;
	diagnostics: readonly LintDiagnostic[];
};

export type WatchMessage = RunStarted | FileResult | Snapshot;

export function parseWatchMessage(line: string): WatchMessage {
	const value: unknown = JSON.parse(line) as unknown;
	const record = asRecord(value, "message");
	if (integer(record.schema_version, "schema_version") !== 1) {
		throw new Error("unsupported schema_version");
	}
	const kind = text(record.kind, "kind");
	if (kind === "run_started") return parseStarted(record);
	if (kind === "file_result") return parseFileResult(record);
	if (kind === "snapshot") return parseSnapshot(record);
	throw new Error(`unknown message kind ${JSON.stringify(kind)}`);
}

function parseFileResult(record: Record<string, unknown>): FileResult {
	const result = {
		schemaVersion: 1,
		kind: "file_result",
		sequence: nonnegativeInteger(record.sequence, "sequence"),
		root: text(record.root, "root"),
		path: text(record.path, "path"),
		diagnostics: array(record.diagnostics, "diagnostics").map(parseDiagnostic),
	} satisfies FileResult;
	if (
		result.diagnostics.some((diagnostic) => diagnostic.path !== result.path)
	) {
		throw new Error("file_result contains a diagnostic for another path");
	}
	return result;
}

function parseStarted(record: Record<string, unknown>): RunStarted {
	const trigger = text(record.trigger, "trigger");
	if (trigger !== "initial" && trigger !== "filesystem") {
		throw new Error(`invalid trigger ${JSON.stringify(trigger)}`);
	}
	return {
		schemaVersion: 1,
		kind: "run_started",
		sequence: nonnegativeInteger(record.sequence, "sequence"),
		trigger,
		changedPaths: array(record.changed_paths, "changed_paths").map((value) =>
			text(value, "changed_paths[]"),
		),
	};
}

function parseSnapshot(record: Record<string, unknown>): Snapshot {
	const status = text(record.status, "status");
	if (!isSnapshotStatus(status)) {
		throw new Error(`invalid snapshot status ${JSON.stringify(status)}`);
	}
	if (integer(record.line_base, "line_base") !== 1) {
		throw new Error("unsupported line_base");
	}
	return {
		schemaVersion: 1,
		kind: "snapshot",
		sequence: nonnegativeInteger(record.sequence, "sequence"),
		root: text(record.root, "root"),
		lineBase: 1,
		status,
		fullUpdate: boolean(record.full_update, "full_update"),
		updatedPaths: array(record.updated_paths, "updated_paths").map((value) =>
			text(value, "updated_paths[]"),
		),
		diagnostics: array(record.diagnostics, "diagnostics").map(parseDiagnostic),
		stats: nullable(record.stats, parseStats),
		precondition: nullable(record.precondition, parsePrecondition),
		error: nullable(record.error, (value) => text(value, "error")),
	};
}

function parseDiagnostic(value: unknown): LintDiagnostic {
	const record = asRecord(value, "diagnostic");
	const severity = text(record.severity, "diagnostic.severity");
	if (severity !== "error" && severity !== "warning") {
		throw new Error(`invalid diagnostic severity ${JSON.stringify(severity)}`);
	}
	return {
		path: text(record.path, "diagnostic.path"),
		ruleId: text(record.rule_id, "diagnostic.rule_id"),
		rulePath: text(record.rule_path, "diagnostic.rule_path"),
		message: text(record.message, "diagnostic.message"),
		severity,
		confidence: probability(record.confidence, "diagnostic.confidence"),
		regions: array(record.regions, "diagnostic.regions").map(parseRegion),
	};
}

function parseRegion(value: unknown): Region {
	const record = asRecord(value, "region");
	const startLine = positiveInteger(record.start_line, "region.start_line");
	const endLine = positiveInteger(record.end_line, "region.end_line");
	if (endLine < startLine) throw new Error("region ends before it starts");
	return { startLine, endLine };
}

function parseStats(value: unknown): SnapshotStats {
	const record = asRecord(value, "stats");
	return {
		filesChecked: nonnegativeInteger(
			record.files_checked,
			"stats.files_checked",
		),
		rules: nonnegativeInteger(record.rules, "stats.rules"),
		apiRequests: nonnegativeInteger(record.api_requests, "stats.api_requests"),
		cacheHits: nonnegativeInteger(record.cache_hits, "stats.cache_hits"),
		violations: nonnegativeInteger(record.violations, "stats.violations"),
		errors: nonnegativeInteger(record.errors, "stats.errors"),
		warnings: nonnegativeInteger(record.warnings, "stats.warnings"),
	};
}

function parsePrecondition(value: unknown): PreconditionStatus {
	const record = asRecord(value, "precondition");
	return {
		command: array(record.command, "precondition.command").map((value) =>
			text(value, "precondition.command[]"),
		),
		exitCode: nullable(record.exit_code, (value) =>
			integer(value, "precondition.exit_code"),
		),
		output: text(record.output, "precondition.output"),
		skippedFiles: array(record.skipped_files, "precondition.skipped_files").map(
			(value) => text(value, "precondition.skipped_files[]"),
		),
	};
}

function asRecord(value: unknown, name: string): Record<string, unknown> {
	if (typeof value !== "object" || value === null || Array.isArray(value)) {
		throw new Error(`${name} must be an object`);
	}
	return value as Record<string, unknown>;
}

function array(value: unknown, name: string): readonly unknown[] {
	if (!Array.isArray(value)) throw new Error(`${name} must be an array`);
	return value;
}

function text(value: unknown, name: string): string {
	if (typeof value !== "string") throw new Error(`${name} must be a string`);
	return value;
}

function boolean(value: unknown, name: string): boolean {
	if (typeof value !== "boolean") throw new Error(`${name} must be a boolean`);
	return value;
}

function integer(value: unknown, name: string): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value)) {
		throw new Error(`${name} must be an integer`);
	}
	return value;
}

function nonnegativeInteger(value: unknown, name: string): number {
	const result = integer(value, name);
	if (result < 0) throw new Error(`${name} must not be negative`);
	return result;
}

function positiveInteger(value: unknown, name: string): number {
	const result = integer(value, name);
	if (result < 1) throw new Error(`${name} must be positive`);
	return result;
}

function probability(value: unknown, name: string): number {
	if (
		typeof value !== "number" ||
		!Number.isFinite(value) ||
		value < 0 ||
		value > 1
	) {
		throw new Error(`${name} must be between zero and one`);
	}
	return value;
}

function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
	return value === null ? null : parse(value);
}

function isSnapshotStatus(value: string): value is SnapshotStatus {
	return (
		value === "clean" ||
		value === "violations" ||
		value === "precondition_failed" ||
		value === "error"
	);
}
