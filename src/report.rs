//! Markdown rendering of a run report.
//!
//! Fluxer and most chat clients render GitHub-Flavored Markdown, so a run can be
//! pasted as a table instead of raw JSON. Cell contents are escaped so an actor name
//! or event containing a pipe cannot break the table.

use crate::model::SimulationReport;

fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

/// Renders one run as GitHub-Flavored Markdown tables.
pub fn render_markdown(report: &SimulationReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "**{} раунд(ов)** · правитель: **{}** · смертей: **{}** · смен трона: **{}** · стабильность: **{}**\n\n",
        report.rounds,
        cell(report.ruler.as_deref().unwrap_or("—")),
        report.deaths,
        report.ruler_changes,
        report.final_stability
    ));

    out.push_str("| раунд | стабильность | правитель | события |\n");
    out.push_str("|---:|---:|---|---|\n");
    for round in &report.history {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            round.round,
            round.stability,
            cell(round.ruler.as_deref().unwrap_or("—")),
            cell(&round.events.join("; "))
        ));
    }

    out.push('\n');
    out.push_str(
        "| актор | политика | жив | сила | ходов | переворотов | казней | убил | убит |\n",
    );
    out.push_str("|---|---|:--:|---:|---:|---:|---:|---|---|\n");
    for actor in &report.actors {
        let kills = if actor.kills.is_empty() {
            String::from("—")
        } else {
            actor.kills.join(", ")
        };
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            cell(&actor.name),
            cell(&actor.policy),
            if actor.alive { "да" } else { "нет" },
            actor.power,
            actor.acts,
            actor.grabs,
            actor.purges,
            cell(&kills),
            cell(actor.killed_by.as_deref().unwrap_or("—"))
        ));
    }
    let explained: Vec<(&str, &crate::model::DecisionReport)> = report
        .actors
        .iter()
        .flat_map(|actor| {
            actor
                .decisions
                .iter()
                .map(move |decision| (actor.name.as_str(), decision))
        })
        .filter(|(_, decision)| decision.note.is_some())
        .collect();
    if !explained.is_empty() {
        out.push_str("\n| актор | раунд | действие | почему |\n");
        out.push_str("|---|---:|---|---|\n");
        for (name, decision) in explained {
            let target = decision
                .target
                .as_deref()
                .map(|target| format!(" → {target}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "| {} | {} | `{}{}` | {} |\n",
                cell(name),
                decision.round,
                cell(&decision.action),
                cell(&target),
                cell(decision.note.as_deref().unwrap_or("—"))
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActorReport, RoundReport};

    fn report() -> SimulationReport {
        SimulationReport {
            rounds: 1,
            final_stability: 4,
            ruler: Some(String::from("big")),
            alive: 2,
            deaths: 1,
            ruler_changes: 1,
            history: vec![RoundReport {
                round: 1,
                stability: 4,
                ruler: Some(String::from("big")),
                alive: vec![String::from("big"), String::from("a|b")],
                events: vec![
                    String::from("big took the throne"),
                    String::from("big purged a|b"),
                ],
            }],
            actors: vec![
                ActorReport {
                    name: String::from("big"),
                    policy: String::from("llm:glm-5.4"),
                    alive: true,
                    power: 5,
                    acts: 1,
                    grabs: 1,
                    purges: 1,
                    kills: vec![String::from("a|b")],
                    killed_by: None,
                    decisions: Vec::new(),
                },
                ActorReport {
                    name: String::from("a|b"),
                    policy: String::from("cautious"),
                    alive: false,
                    power: 0,
                    acts: 0,
                    grabs: 0,
                    purges: 0,
                    kills: Vec::new(),
                    killed_by: Some(String::from("big")),
                    decisions: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn test_should_render_reasoning_as_a_separate_table() {
        let mut report = report();
        report.actors[0]
            .decisions
            .push(crate::model::DecisionReport {
                round: 1,
                action: String::from("grab-power"),
                target: None,
                note: Some(String::from("stability is high, power is cheap")),
            });

        let text = render_markdown(&report);

        assert!(text.contains("| актор | раунд | действие | почему |"));
        assert!(text.contains("stability is high, power is cheap"));
        assert!(text.contains("| big | 1 | `grab-power` |"));
    }

    #[test]
    fn test_should_render_gfm_tables_with_escaped_cells() {
        let text = render_markdown(&report());

        assert!(text.contains("| раунд | стабильность | правитель | события |"));
        assert!(text.contains("| актор | политика | жив | сила | ходов |"));
        assert!(text.contains("| 1 | 4 | big | big took the throne; big purged a\\|b |"));
        assert!(
            !text.contains("| a|b |"),
            "a pipe inside a cell must be escaped"
        );
        assert!(text.contains("a\\|b"));
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("| 1 |"))
                .count(),
            1
        );
    }
}
