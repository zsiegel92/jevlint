use std::io::{self, Write};

use crate::engine::{LineRegion, RunReport, Violation};
use crate::rule::Severity;

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
    print_violations(&report.violations, &mut output)?;
    print_summary(report, output)
}

pub fn print_violations(violations: &[Violation], mut output: impl Write) -> io::Result<()> {
    for violation in violations {
        if violation.regions.is_empty() {
            writeln!(
                output,
                "{}: {} [{}] {} (rule: {}, confidence: {:.2})",
                violation.path.display(),
                violation.severity,
                violation.rule_id,
                compact_message(&violation.message),
                violation.rule_path.display(),
                violation.confidence
            )?;
        } else {
            for region in &violation.regions {
                writeln!(
                    output,
                    "{}:{}: {} [{}] {} (rule: {}, confidence: {:.2})",
                    violation.path.display(),
                    display_region(region),
                    violation.severity,
                    violation.rule_id,
                    compact_message(&violation.message),
                    violation.rule_path.display(),
                    violation.confidence
                )?;
            }
        }
    }
    Ok(())
}

pub fn print_summary(report: &RunReport, mut output: impl Write) -> io::Result<()> {
    writeln!(
        output,
        "jevlint: {} files, {} rules, {} API requests, {} cached, {} errors, {} warnings",
        report.files_checked,
        report.rules,
        report.api_requests,
        report.cache_hits,
        report.count(Severity::Error),
        report.count(Severity::Warning)
    )
}

fn compact_message(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_region(region: &LineRegion) -> String {
    if region.start == region.end {
        region.start.to_string()
    } else {
        format!("{}-{}", region.start, region.end)
    }
}
