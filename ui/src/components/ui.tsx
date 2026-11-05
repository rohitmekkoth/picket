import type { ReactNode } from "react";
import type { Level } from "../lib/api";
import { levelHelp, levelLabel } from "../lib/copy";

// Small shared pieces. Icons are inline strokes so they follow the text color.

const paths = {
  home: <path d="M3 8l6-5 6 5v7H3z" strokeLinejoin="round" />,
  review: (
    <>
      <path d="M9 2.5l7 12.5H2z" strokeLinejoin="round" />
      <path d="M9 7.5v3M9 12.6v.1" strokeLinecap="round" />
    </>
  ),
  activity: <path d="M3 4.5h12M3 9h12M3 13.5h8" strokeLinecap="round" />,
  rules: (
    <>
      <path d="M3 5h5M12 5h3M3 13h2M9 13h6" strokeLinecap="round" />
      <circle cx="10" cy="5" r="2" />
      <circle cx="7" cy="13" r="2" />
    </>
  ),
  settings: (
    <>
      <circle cx="9" cy="9" r="2.4" />
      <path
        d="M9 1.8v2.1M9 14.1v2.1M1.8 9h2.1M14.1 9h2.1M3.9 3.9l1.5 1.5M12.6 12.6l1.5 1.5M3.9 14.1l1.5-1.5M12.6 5.4l1.5-1.5"
        strokeLinecap="round"
      />
    </>
  ),
  mail: (
    <>
      <rect x="2.5" y="4" width="13" height="10" rx="1.8" />
      <path d="M3 5l6 4.5L15 5" strokeLinejoin="round" />
    </>
  ),
  calendar: (
    <>
      <rect x="2.5" y="3.5" width="13" height="11.5" rx="1.8" />
      <path d="M2.5 7.5h13M6 2v3M12 2v3" strokeLinecap="round" />
    </>
  ),
  chat: (
    <path
      d="M3.5 3.5h11a1 1 0 011 1v7a1 1 0 01-1 1H8l-3.5 3v-3h-1a1 1 0 01-1-1v-7a1 1 0 011-1z"
      strokeLinejoin="round"
    />
  ),
  moon: <path d="M14.5 10.5A6 6 0 017.5 3.5a6 6 0 107 7z" strokeLinejoin="round" />,
  check: <path d="M4 9.4l3.2 3.2L14 5.6" strokeLinecap="round" strokeLinejoin="round" />,
  warning: (
    <>
      <path d="M9 2.8l6.6 12H2.4z" strokeLinejoin="round" />
      <path d="M9 7.4v3M9 12.8v.1" strokeLinecap="round" />
    </>
  ),
  chevron: <path d="M7 4l5 5-5 5" strokeLinecap="round" strokeLinejoin="round" />,
  back: <path d="M11 4L6 9l5 5" strokeLinecap="round" strokeLinejoin="round" />,
  plus: <path d="M9 4v10M4 9h10" strokeLinecap="round" />,
  search: (
    <>
      <circle cx="8" cy="8" r="5" />
      <path d="M12 12l3.5 3.5" strokeLinecap="round" />
    </>
  ),
};

export type IconName = keyof typeof paths;

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg
      className="icon"
      width={size}
      height={size}
      viewBox="0 0 18 18"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}

export function Mark({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 22 22" fill="none" aria-hidden="true">
      <path
        d="M4 20V6l2-3 2 3v14M10 20V6l2-3 2 3v14M16 20V6l2-3 2 3v14"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
      <path d="M2.5 10h17M2.5 15h17" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}

export type Tone = "review" | "urgent" | "ok" | "muted";

export function Badge({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span className={`badge badge-${tone}`}>
      {(tone === "review" || tone === "urgent") && <span className="dot" aria-hidden="true" />}
      {children}
    </span>
  );
}

export function LevelControl({
  label,
  value,
  onChange,
  disabled = false,
  allowOff = true,
}: {
  label: string;
  value: Level;
  onChange: (level: Level) => void;
  disabled?: boolean;
  allowOff?: boolean;
}) {
  const options: Level[] = allowOff
    ? ["urgent", "review", "summary", "off"]
    : ["urgent", "review", "summary"];
  return (
    <fieldset className="levels" aria-label={label} disabled={disabled}>
      {options.map((option) => (
        <button
          key={option}
          type="button"
          className={value === option ? `level level-${option}` : "level"}
          aria-pressed={value === option}
          title={levelHelp[option]}
          onClick={() => onChange(option)}
        >
          {levelLabel[option]}
        </button>
      ))}
    </fieldset>
  );
}

export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <fieldset className="segmented" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          className={value === option.value ? "on" : undefined}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </fieldset>
  );
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <p className="empty-title">{title}</p>
      {children && <p className="empty-text">{children}</p>}
    </div>
  );
}

export function DesktopOnly() {
  return (
    <Empty title="Open the desktop app">
      This preview can’t reach Picket’s core, so there is nothing to show here.
    </Empty>
  );
}

export function Failure({ error }: { error: unknown }) {
  return (
    <div className="notice notice-error" role="alert">
      {error instanceof Error ? error.message : String(error)}
    </div>
  );
}
