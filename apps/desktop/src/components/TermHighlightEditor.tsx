import {
  DEFAULT_TERM_HIGHLIGHTS,
  HIGHLIGHT_PRESETS,
  SCHEME_LABELS,
  newHighlightRule,
  previewHighlights,
  type HighlightSchemeId,
  type TermHighlightSettings,
} from "../termHighlight";

const SCHEME_IDS = ["down", "up", "warn"] as const;

export function TermHighlightEditor({
  value,
  onChange,
}: {
  value: TermHighlightSettings;
  onChange: (next: TermHighlightSettings) => void;
}) {
  const preview = previewHighlights(value);
  return (
    <div className="hl-block">
      <h3>Keyword highlights</h3>
      <p className="hint">
        Same idea as SecureCRT: color words in SSH, serial, and local terminals. Change the{" "}
        <strong>Down</strong> and <strong>Up</strong> colors below — every keyword in that scheme follows. Display
        only; logs and the agent still see the raw text. Saved on this computer.
      </p>
      <label className="check">
        <input
          type="checkbox"
          checked={value.enabled}
          onChange={(e) => onChange({ ...value, enabled: e.target.checked })}
        />
        Enable keyword highlights
      </label>
      <div className="hl-schemes">
        {SCHEME_IDS.map((id) => (
          <div key={id} className="hl-scheme">
            <strong>{SCHEME_LABELS[id]}</strong>
            <ColorField
              fg={value.schemes[id].fg}
              bg={value.schemes[id].bg}
              onFg={(fg) => onChange({ ...value, schemes: { ...value.schemes, [id]: { ...value.schemes[id], fg } } })}
              onBg={(bg) =>
                onChange({
                  ...value,
                  schemes: { ...value.schemes, [id]: bg ? { ...value.schemes[id], bg } : { fg: value.schemes[id].fg } },
                })
              }
            />
          </div>
        ))}
      </div>
      <div className="hl-head">
        <span>Keyword</span>
        <span>Scheme</span>
        <span title="Only match a whole word (up in uptime is skipped)">Word</span>
        <span title="Match case">Aa</span>
        <span />
      </div>
      <div className="hl-rows">
        {value.rules.map((r, i) => (
          <div key={r.id} className="hl-row">
            <input
              value={r.pattern}
              placeholder="down"
              maxLength={64}
              aria-label={`Keyword ${i + 1}`}
              onChange={(e) => patchRule(value, onChange, r.id, { pattern: e.target.value })}
            />
            <div className="hl-scheme-cell">
              <select
                value={r.scheme}
                aria-label={`Scheme for ${r.pattern || "keyword"}`}
                onChange={(e) => {
                  const scheme = e.target.value as HighlightSchemeId;
                  const fg = scheme === "custom" ? r.fg : value.schemes[scheme].fg;
                  patchRule(value, onChange, r.id, { scheme, fg });
                }}
              >
                <option value="down">Down</option>
                <option value="up">Up</option>
                <option value="warn">Warn</option>
                <option value="custom">Custom</option>
              </select>
              {r.scheme === "custom" && (
                <ColorField fg={r.fg} bg={r.bg} onFg={(fg) => patchRule(value, onChange, r.id, { fg })} onBg={(bg) => patchRule(value, onChange, r.id, { bg })} compact />
              )}
            </div>
            <label className="hl-flag" title="Whole word">
              <input
                type="checkbox"
                checked={r.wholeWord}
                onChange={(e) => patchRule(value, onChange, r.id, { wholeWord: e.target.checked })}
              />
            </label>
            <label className="hl-flag" title="Match case">
              <input
                type="checkbox"
                checked={r.caseSensitive}
                onChange={(e) => patchRule(value, onChange, r.id, { caseSensitive: e.target.checked })}
              />
            </label>
            <button
              type="button"
              className="ghost"
              title="Remove"
              onClick={() => onChange({ ...value, rules: value.rules.filter((x) => x.id !== r.id) })}
            >
              ×
            </button>
          </div>
        ))}
      </div>
      <div className="hl-actions">
        <button
          type="button"
          className="ghost"
          onClick={() => onChange({ ...value, rules: [...value.rules, newHighlightRule()] })}
        >
          Add keyword
        </button>
        <button type="button" className="ghost" onClick={() => onChange(structuredClone(DEFAULT_TERM_HIGHLIGHTS))}>
          Restore defaults
        </button>
      </div>
      <pre className="hl-preview" aria-label="Highlight preview">
        {ansiToPreview(preview)}
      </pre>
    </div>
  );
}

function ColorField({
  fg,
  bg,
  onFg,
  onBg,
  compact,
}: {
  fg: string;
  bg?: string;
  onFg: (fg: string) => void;
  onBg: (bg: string | undefined) => void;
  compact?: boolean;
}) {
  return (
    <div className={compact ? "hl-color compact" : "hl-color"}>
      <input type="color" value={fg} aria-label="Foreground" onChange={(e) => onFg(e.target.value)} />
      <span className="hl-swatches">
        {HIGHLIGHT_PRESETS.map((p) => (
          <button
            key={p.fg}
            type="button"
            className={fg === p.fg ? "on" : ""}
            title={p.name}
            style={{ background: p.fg }}
            onClick={() => onFg(p.fg)}
          />
        ))}
      </span>
      <label className="hl-flag" title="Background fill">
        <input
          type="checkbox"
          checked={Boolean(bg)}
          onChange={(e) => onBg(e.target.checked ? bg || "#3a1520" : undefined)}
        />
        fill
      </label>
      {bg ? <input type="color" value={bg} aria-label="Background" onChange={(e) => onBg(e.target.value)} /> : null}
    </div>
  );
}

function patchRule(
  value: TermHighlightSettings,
  onChange: (next: TermHighlightSettings) => void,
  id: string,
  patch: Partial<TermHighlightSettings["rules"][number]>,
) {
  onChange({
    ...value,
    rules: value.rules.map((r) => (r.id === id ? { ...r, ...patch } : r)),
  });
}

function ansiToPreview(s: string): JSX.Element[] {
  const parts: JSX.Element[] = [];
  const re = /\x1b\[38;2;(\d+);(\d+);(\d+)m([\s\S]*?)\x1b\[39m/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(s))) {
    if (m.index > last) parts.push(<span key={`t${k++}`}>{s.slice(last, m.index)}</span>);
    parts.push(
      <span key={`c${k++}`} style={{ color: `rgb(${m[1]},${m[2]},${m[3]})`, fontWeight: 700 }}>
        {stripSgr(m[4])}
      </span>,
    );
    last = m.index + m[0].length;
  }
  if (last < s.length) parts.push(<span key={`t${k++}`}>{stripSgr(s.slice(last))}</span>);
  if (!parts.length) parts.push(<span key="empty">{stripSgr(s)}</span>);
  return parts;
}

function stripSgr(s: string): string {
  return s.replace(/\x1b\[[0-9;]*m/g, "");
}
