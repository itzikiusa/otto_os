import React from "react";
import type { Chapter } from "../model";
import { font, mono, palette as p, tween } from "../design";
export const chip = (color = "#246653"): React.CSSProperties => ({
  display: "inline-flex",
  alignItems: "center",
  gap: 9,
  padding: "8px 13px",
  borderRadius: 7,
  background: color + "13",
  color,
  fontSize: 20,
  fontWeight: 600,
});
export const panel: React.CSSProperties = {
  background: "white",
  border: `1px solid ${p.line}`,
  borderRadius: 12,
  overflow: "hidden",
};

export function Dot({ color = p.mint }: { color?: string }) {
  return (
    <span
      style={{
        display: "inline-block",
        width: 8,
        height: 8,
        borderRadius: "50%",
        background: color,
        flexShrink: 0,
      }}
    />
  );
}
export function Code({
  lines,
  dark = false,
  reveal = 100,
}: {
  lines: string[];
  dark?: boolean;
  reveal?: number;
}) {
  return (
    <div
      style={{
        fontFamily: mono,
        fontSize: 22,
        lineHeight: 1.9,
        whiteSpace: "pre-wrap",
        color: dark ? "#ccd9eb" : p.ink,
      }}
    >
      {lines.slice(0, reveal).map((line, i) => (
        <div key={i} style={{ display: "flex", gap: 24 }}>
          <span style={{ opacity: 0.3, minWidth: 24, textAlign: "right" }}>
            {i + 1}
          </span>
          <span
            style={{
              color: line.startsWith("+")
                ? "#329a79"
                : line.startsWith("-")
                  ? "#be626a"
                  : undefined,
            }}
          >
            {line}
          </span>
        </div>
      ))}
    </div>
  );
}
export function Rows({
  items,
  phase,
  columns = false,
}: {
  items: string[];
  phase: number;
  columns?: boolean;
}) {
  return (
    <div style={panel}>
      {items.map((item, i) => (
        <div
          key={item}
          style={{
            padding: "25px 25px",
            borderBottom:
              i < items.length - 1 ? `1px solid ${p.line}` : undefined,
            background: i === phase ? "#eaf2ff" : "white",
            display: "flex",
            alignItems: "center",
            gap: 18,
            minHeight: 72,
          }}
        >
          <span
            style={{ color: i === phase ? "#3975c5" : "#8695a9", fontSize: 22 }}
          >
            {i === phase ? "●" : "○"}
          </span>
          {item.split(" / ").map((part, j) => (
            <span
              key={j}
              style={{
                fontSize: 23,
                minWidth: 0,
                overflowWrap: "anywhere",
                fontWeight: j === 0 ? 600 : 450,
                flex: columns ? 1 : undefined,
                color: j > 0 ? p.muted : p.ink,
              }}
            >
              {part}
            </span>
          ))}
        </div>
      ))}
    </div>
  );
}
