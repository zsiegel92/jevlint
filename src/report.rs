use std::io::{self, Write};

use crate::engine::{LineRegion, RunReport};

pub fn print(report: &RunReport, mut output: impl Write, mut errors: impl Write) -> io::Result<()> {
    if let Some(failure) = &report.precondition_failure {
        writeln!(
            errors,
            "jevlint: precondition failed (exit {})",
            failure
                .status
                .map_or_else(|| "signal".into(), |x| x.to_string())
        )?;
        writeln!(errors, "command: {}", failure.command.join(" "))?;
        if !failure.output.is_empty() {
            writeln!(errors, "{}", failure.output)?;
        }
        for path in &failure.files {
            writeln!(errors, "{}: skipped (precondition failed)", path.display())?;
        }
        return Ok(());
    }
    for violation in &report.violations {
        if violation.regions.is_empty() {
            writeln!(
                output,
                "{}: error [{}] ({:.2})",
                violation.path.display(),
                violation.rule_id,
                violation.confidence
            )?;
        } else {
            for region in &violation.regions {
                writeln!(
                    output,
                    "{}:{}: error [{}] ({:.2})",
                    violation.path.display(),
                    display_region(region),
                    violation.rule_id,
                    violation.confidence
                )?;
            }
        }
    }
    writeln!(
        output,
        "jevlint: {} files, {} rules, {} API requests, {} cached, {} violations",
        report.files_checked,
        report.rules,
        report.api_requests,
        report.cache_hits,
        report.violations.len()
    )
}

fn display_region(region: &LineRegion) -> String {
    if region.start == region.end {
        region.start.to_string()
    } else {
        format!("{}-{}", region.start, region.end)
    }
}
