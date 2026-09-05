import React from "react";
import type { Chapter } from "../model";
import { font, mono, palette as p, tween } from "../design";
import { chip, panel, Code, Rows } from "./primitives";
export function DataDemo({ ch, phase }: { ch: Chapter; phase: number }) {
  const query = ch.view === "database";
  const api = ch.view === "api";
  return (
    <div style={panel}>
      {(query || api) && (
        <div
          style={{
            background: p.ink,
            color: "#dbe7f7",
            padding: "24px 28px",
            fontFamily: mono,
            fontSize: 22,
            lineHeight: 1.7,
          }}
        >
          {query ? (
            <>
              {"SELECT checkout_id, state, attempts"}
              <br />
              {"FROM checkouts WHERE state != 'succeeded';"}
            </>
          ) : (
            <>
              <span style={{ color: p.mint }}>GET</span>{" "}
              https://api.example.com/checkouts/co_1043
            </>
          )}
          <div style={{ marginTop: 18, fontFamily: font, ...chip("#72d4be") }}>
            {phase > 0
              ? query
                ? "Query complete"
                : "200 OK"
              : query
                ? "Run query"
                : "Send request"}
          </div>
        </div>
      )}
      {api ? (
        <div style={{ padding: 28 }}>
          <Code
            lines={
              phase > 0
                ? [
                    "{",
                    '  "checkout_id": "co_1043",',
                    '  "state": "retrying",',
                    '  "attempts": 2',
                    "}",
                  ]
                : ["// Response appears here after Send"]
            }
          />
        </div>
      ) : (
        <>
          <div
            style={{
              padding: "17px 24px",
              fontSize: 19,
              color: p.muted,
              borderBottom: `1px solid ${p.line}`,
            }}
          >
            {query
              ? "Results / 2 rows"
              : ch.view === "events"
                ? "Messages / Key · Headers · Value"
                : "Profiles / Name · Type · Status"}
          </div>
          <Rows
            items={query ? ch.items.slice(2) : ch.items}
            phase={phase}
            columns
          />
          {ch.view === "events" && phase > 0 && (
            <div
              style={{
                padding: 24,
                background: "#f6f8fc",
                fontFamily: mono,
                fontSize: 20,
              }}
            >
              {'{ "checkout_id": "co_1043", "event": "payment.retried" }'}
            </div>
          )}
        </>
      )}
    </div>
  );
}
export function Metrics({
  ch,
  phase,
  frame,
}: {
  ch: Chapter;
  phase: number;
  frame: number;
}) {
  const points = [
    180, 173, 165, 178, 150, 130, 140, 95, 106, 80, 89, 71, 92, 130, 100, 113,
    80, 76, 91, 68, 59, 72,
  ];
  const d = points
    .map((v, i) => `${i === 0 ? "M" : "L"}${70 + i * 43},${v}`)
    .join(" ");
  return (
    <div style={panel}>
      <div
        style={{
          padding: 25,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <div style={{ fontSize: 24, fontWeight: 600 }}>
          {ch.id === "usage" ? "Usage over time" : "CPU utilization"}
        </div>
        <span style={chip("#4969a6")}>
          {phase === 0 ? "Last hour" : "Last 24 hours"}
        </span>
      </div>
      <svg viewBox="0 0 1080 290" width="100%" height="280">
        <defs>
          <linearGradient id="area" x1="0" y1="0" x2="0" y2="1">
            <stop stopColor="#6e9fdc" stopOpacity=".25" />
            <stop offset="1" stopColor="#6e9fdc" stopOpacity="0" />
          </linearGradient>
        </defs>
        {[50, 110, 170, 230].map((y) => (
          <g key={y}>
            <line x1="65" x2="1030" y1={y} y2={y} stroke={p.line} />
            <text
              x="15"
              y={y + 6}
              fill={p.muted}
              fontFamily={font}
              fontSize="18"
            >
              {100 - Math.round(y / 3)}
            </text>
          </g>
        ))}
        <path d={d + " L973,230 L70,230 Z"} fill="url(#area)" />
        <path
          d={d}
          pathLength="1"
          strokeDasharray="1"
          strokeDashoffset={1 - tween(frame, 15, 85)}
          stroke="#5e94d5"
          strokeWidth="4"
          fill="none"
        />
        <text x="70" y="270" fill={p.muted} fontSize="18">
          00:00
        </text>
        <text x="500" y="270" fill={p.muted} fontSize="18">
          12:00
        </text>
        <text x="940" y="270" fill={p.muted} fontSize="18">
          23:59
        </text>
      </svg>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          borderTop: `1px solid ${p.line}`,
        }}
      >
        {ch.items.map((t) => (
          <div
            key={t}
            style={{ fontSize: 21, padding: "20px 25px", color: p.muted }}
          >
            {t}
          </div>
        ))}
      </div>
    </div>
  );
}
export function Cloud({ ch, phase }: { ch: Chapter; phase: number }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: ch.view === "cloud" ? "260px 1fr" : "1fr",
        gap: 22,
      }}
    >
      {ch.view === "cloud" && (
        <div style={{ ...panel, padding: 25 }}>
          <div style={{ fontSize: 20, color: p.muted }}>Account</div>
          <h3 style={{ fontSize: 26, marginTop: 16 }}>Atlas staging</h3>
          <div style={chip()}>● Signed in</div>
          <div style={{ fontSize: 20, color: p.muted, marginTop: 30 }}>
            Region
          </div>
          <div style={{ fontSize: 23, marginTop: 10 }}>eu-west-1</div>
        </div>
      )}
      <div>
        {ch.view === "kubernetes" && (
          <div style={{ display: "flex", gap: 15, marginBottom: 22 }}>
            <span style={chip("#4969a6")}>Cluster: Atlas staging</span>
            <span style={chip("#4969a6")}>Namespace: checkout</span>
          </div>
        )}
        <Rows items={ch.items} phase={phase} />
        {ch.view === "kubernetes" && phase > 0 && (
          <div
            style={{
              ...panel,
              padding: 22,
              marginTop: 18,
              display: "flex",
              gap: 24,
            }}
          >
            {["Details", "Manifest", "Events", "Pods", "Logs"].map((t) => (
              <span
                key={t}
                style={{
                  fontSize: 22,
                  color: (phase === 1 ? t === "Pods" : t === "Logs")
                    ? "#3975c5"
                    : p.muted,
                }}
              >
                {t}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
