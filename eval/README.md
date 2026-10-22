# Mock evaluation

Run `make eval` or `picket eval --mock --report path/to/report.json`. The command replays fixed synthetic events through the same SQLite store and deterministic engine used by the product. It prints a mode table, writes a JSON report, and exits unsuccessfully when a gate fails. CI runs it on the portable core job.

`scenarios/held_out.json` fixes 18 scenario families with 16 variants each. Variants change synthetic identities, resources, live/catch-up origin, and event arrival order. The generator in `crates/core/src/eval.rs` keeps truth labels separate from events sent to the engine. `baseline.json` fixes minimum denominators and regression thresholds. The 24 M3 seed cases remain separate unit tests.

The report counts evaluated effects, named-agent assertions, human false blame, linkage, unknown actors, explicit request conflicts, alert results by severity, benign alert burden, and surfaced coverage gaps. Zero denominators are visible. A fully abstaining engine fails linkage and conflict recall gates. The report contains no message text, email addresses, or credentials.

These are correlated, constructed cases. They test implementation regressions, not population accuracy. Confidence intervals, real-model extraction, source behavior, latency, and alerts per day are `not_measured` until later milestones provide independent labeled data. Do not present the mock pass rate as a production reliability claim.
