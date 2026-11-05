import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useAgentName } from "../App";
import { DesktopOnly, Failure, Icon, LevelControl } from "../components/ui";
import { getRules, type Level, levels, setRuleLevel } from "../lib/api";
import { describeBoundary, levelLabel, levelShort, ruleGroups } from "../lib/copy";
import { href } from "../lib/router";

function recentText(count: number): string {
  if (count === 0) return "Would have flagged nothing in the last 30 days";
  return `Would have flagged ${count} in the last 30 days`;
}

export function Rules() {
  const client = useQueryClient();
  const agent = useAgentName();
  const [error, setError] = useState<unknown>(null);
  const rules = useQuery({ queryKey: ["rules"], queryFn: getRules });
  const change = useMutation({
    mutationFn: ({ id, value }: { id: string; value: Level }) => setRuleLevel(id, value),
    onMutate: ({ id, value }) => {
      // Show the choice at once; the next fetch confirms what was saved.
      client.setQueryData(["rules"], (current: typeof rules.data) =>
        current
          ? {
              ...current,
              levels: current.levels.map((item) =>
                item.id === id ? { ...item, level: value } : item,
              ),
            }
          : current,
      );
    },
    onSuccess: () => setError(null),
    onError: setError,
    onSettled: () => client.invalidateQueries({ queryKey: ["rules"] }),
  });
  if (rules.isSuccess && rules.data === null) return <DesktopOnly />;
  const data = rules.data;
  const level = (id: string): Level => data?.levels.find((item) => item.id === id)?.level ?? "off";
  const name = agent ?? "your agent";
  return (
    <div className="stack stack-wide">
      <header className="page-header">
        <div>
          <h1>Rules</h1>
          <p className="lede">
            Choose what Picket tells you about, and how. Everything is still recorded in Activity.
          </p>
        </div>
      </header>
      <div className="card legend">
        {levels.map((item) => (
          <div key={item} className="legend-item">
            <span className={`swatch swatch-${item}`} aria-hidden="true" />
            <span>
              <span className="strong">{levelLabel[item]}</span>
              <span className="caption">{levelShort[item]}</span>
            </span>
          </div>
        ))}
      </div>
      {error !== null && <Failure error={error} />}

      {ruleGroups.slice(0, 1).map((group) => (
        <RuleSection
          key={group.title}
          group={group}
          name={name}
          level={level}
          onChange={change.mutate}
        />
      ))}

      <section className="rule-section" aria-labelledby="boundaries-heading">
        <div className="section-head">
          <div>
            <h2 id="boundaries-heading">Your boundaries</h2>
            <p className="section-note">
              Lines that should never be crossed. They apply to everything in your Gmail and
              Calendar, even when Picket can’t tell who did it.
            </p>
          </div>
          <a className="button" href={href({ page: "boundary", id: null })}>
            <Icon name="plus" size={14} />
            Add a boundary
          </a>
        </div>
        <div className="card list">
          {data && data.boundaries.length === 0 ? (
            <p className="list-empty">
              No boundaries yet. Add one to be told about something whoever does it, such as email
              to your bank or deleted calendar events.
            </p>
          ) : (
            data?.boundaries.map((boundary) => (
              <a
                key={boundary.id}
                className="rule-row rule-link"
                href={href({ page: "boundary", id: boundary.id })}
              >
                <span className="rule-text">
                  <span className="boundary-text">{describeBoundary(boundary.kind)}</span>
                  <span className="caption">{recentText(boundary.recent)}</span>
                </span>
                <span className={`level-pill level-${boundary.level}`}>
                  {levelLabel[boundary.level]}
                </span>
                <Icon name="chevron" size={16} />
              </a>
            ))
          )}
        </div>
      </section>

      {ruleGroups.slice(1).map((group) => (
        <RuleSection
          key={group.title}
          group={group}
          name={name}
          level={level}
          onChange={change.mutate}
        />
      ))}
    </div>
  );
}

function RuleSection({
  group,
  name,
  level,
  onChange,
}: {
  group: (typeof ruleGroups)[number];
  name: string;
  level: (id: string) => Level;
  onChange: (change: { id: string; value: Level }) => void;
}) {
  const withName = (text: string) =>
    text.replace(/your agent/g, name).replace(/^./, (first) => first.toUpperCase());
  const headingId = `rules-${group.title.replace(/\W+/g, "-").toLowerCase()}`;
  return (
    <section className="rule-section" aria-labelledby={headingId}>
      <h2 id={headingId}>{withName(group.title)}</h2>
      <p className="section-note">{withName(group.note)}</p>
      <div className="card list">
        {group.groups.map((part) => (
          <div key={part.label ?? group.title} className="rule-group">
            {part.label && <p className="group-label">{part.label}</p>}
            {part.rules.map((rule) => (
              <div key={rule.id} className="rule-row">
                <span className="rule-text">
                  <span className="strong">{withName(rule.name)}</span>
                  <span className="caption">{withName(rule.example)}</span>
                </span>
                <LevelControl
                  label={withName(rule.name)}
                  value={level(rule.id)}
                  onChange={(value) => onChange({ id: rule.id, value })}
                />
              </div>
            ))}
          </div>
        ))}
      </div>
    </section>
  );
}
