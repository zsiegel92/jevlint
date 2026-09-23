import assert from "node:assert/strict";
import test from "node:test";
import { parseWatchMessage } from "./protocol.js";

test("parses a warning with a line region", () => {
	const message = parseWatchMessage(
		JSON.stringify({
			schema_version: 1,
			kind: "snapshot",
			sequence: 3,
			root: "/project",
			line_base: 1,
			status: "violations",
			full_update: false,
			updated_paths: ["src/main.ts"],
			diagnostics: [
				{
					path: "src/main.ts",
					rule_id: "clear-boundaries",
					rule_path: ".jevlint-rules/clear-boundaries.md",
					message: "# Keep boundaries clear",
					severity: "warning",
					confidence: 0.91,
					regions: [{ start_line: 4, end_line: 6 }],
				},
			],
			stats: {
				files_checked: 1,
				rules: 1,
				api_requests: 1,
				cache_hits: 0,
				violations: 1,
				errors: 0,
				warnings: 1,
			},
			precondition: null,
			error: null,
		}),
	);
	assert.equal(message.kind, "snapshot");
	assert.equal(message.diagnostics[0]?.severity, "warning");
	assert.equal(message.diagnostics[0]?.message, "# Keep boundaries clear");
	assert.deepEqual(message.diagnostics[0]?.regions[0], {
		startLine: 4,
		endLine: 6,
	});
});

test("rejects an unknown severity", () => {
	assert.throws(() =>
		parseWatchMessage(
			'{"schema_version":1,"kind":"snapshot","sequence":1,"root":"/","line_base":1,"status":"violations","full_update":false,"updated_paths":["x"],"diagnostics":[{"path":"x","rule_id":"r","rule_path":".jevlint-rules/r.md","message":"Rule","severity":"info","confidence":1,"regions":[]}],"stats":null,"precondition":null,"error":null}',
		),
	);
});

test("parses a streamed file result", () => {
	const message = parseWatchMessage(
		JSON.stringify({
			schema_version: 1,
			kind: "file_result",
			sequence: 4,
			root: "/project",
			path: "src/main.ts",
			diagnostics: [
				{
					path: "src/main.ts",
					rule_id: "clear-boundaries",
					rule_path: ".jevlint-rules/clear-boundaries.md",
					message: "# Keep boundaries clear",
					severity: "error",
					confidence: 0.91,
					regions: [{ start_line: 4, end_line: 4 }],
				},
			],
		}),
	);
	assert.equal(message.kind, "file_result");
	assert.equal(message.path, "src/main.ts");
	assert.equal(message.diagnostics[0]?.severity, "error");
});
