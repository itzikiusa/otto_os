import React from "react";
import type { Chapter } from "../model";
import { font, mono, palette as p, tween } from "../design";
import { chip, panel, Code, Rows } from "./primitives";
export function Terminal({
  ch,
  phase,
  frame,
}: {
  ch: Chapter;
  phase: number;
  frame: number;
}) {
  const lines = [
    "$ codex",
    "› Fix the checkout timeout.",
    "",
    "Reading src/checkout/retry.ts",
    "Inspecting the final retry attempt…",
    "",
    "The timeout path needs a bounded retry.",
    "I will add a regression test first.",
  ];
  const shown =
    phase === 0
      ? 2
      : phase === 1
        ? Math.min(6, 3 + Math.floor((frame % 160) / 28))
        : 8;
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: phase === 2 ? "1fr 1fr" : "1fr",
        gap: 18,
        height: 470,
      }}
    >
      <div style={{ background: p.ink, borderRadius: 12, padding: 28 }}>
        <div style={{ color: "#91a4bd", fontSize: 19, marginBottom: 26 }}>
          Codex <span style={{ float: "right", color: p.mint }}>● Running</span>
        </div>
        <Code lines={lines} dark reveal={shown} />
        <span
          style={{
            display: "block",
            marginTop: 20,
            width: 11,
            height: 24,
            background: p.mint,
            opacity: Math.floor(frame / 18) % 2 ? 0.35 : 1,
          }}
        />
      </div>
      {phase === 2 && (
        <div
          style={{
            background: "#19273e",
            borderRadius: 12,
            padding: 28,
            color: "white",
          }}
        >
          <div style={{ fontSize: 19, color: "#91a4bd", marginBottom: 25 }}>
            Shell
          </div>
          <Code
            lines={[
              "$ npm test",
              "",
              "retry.test.ts",
              "✓ transient failure",
              "✓ final attempt",
              "",
              "2 tests passed",
            ]}
            dark
          />
        </div>
      )}
    </div>
  );
}
export function Diff({ phase }: { phase: number }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "260px 1fr", gap: 20 }}>
      <div style={panel}>
        <div style={{ padding: 22, fontWeight: 600, fontSize: 22 }}>
          Changes <span style={{ float: "right", color: p.muted }}>3</span>
        </div>
        {["retry.ts", "retry.test.ts", "checkout.md"].map((f, i) => (
          <div
            key={f}
            style={{
              padding: "19px 22px",
              background: i === 0 ? "#eaf2ff" : "white",
              fontFamily: mono,
              fontSize: 18,
            }}
          >
            {" "}
            <span style={{ color: phase > 0 ? "#329a79" : "#7e91af" }}>
              {phase > 0 && i === 0 ? "✓" : "M"}
            </span>{" "}
            {f}
          </div>
        ))}
        <div style={{ margin: 20, ...chip() }}>
          {phase > 0 ? "1 file staged" : "Select a file"}
        </div>
      </div>
      <div style={panel}>
        <div
          style={{
            padding: 22,
            borderBottom: `1px solid ${p.line}`,
            fontFamily: mono,
            fontSize: 20,
          }}
        >
          src/checkout/retry.ts
        </div>
        <div style={{ padding: 25 }}>
          <Code
            lines={[
              "async function retry(request) {",
              "-  return send(request);",
              "+  for (let attempt = 0;",
              "+       attempt < MAX_RETRIES;",
              "+       attempt++) {",
              "+    if (await send(request)) return;",
              "+    await backoff(attempt);",
              "+  }",
              "}",
            ]}
          />
        </div>
      </div>
    </div>
  );
}
export function Flow({
  ch,
  phase,
  frame,
}: {
  ch: Chapter;
  phase: number;
  frame: number;
}) {
  const isTeam = ch.view === "team";
  const items = ch.items;
  const progress = tween(frame, 30, Math.max(31, ch.duration * 30 - 60));
  return (
    <div
      style={{
        ...panel,
        height: 465,
        position: "relative",
        backgroundImage: "radial-gradient(#d0dbe7 1px, transparent 1px)",
        backgroundSize: "24px 24px",
      }}
    >
      <svg
        width="100%"
        height="100%"
        viewBox="0 0 1080 465"
        style={{ position: "absolute" }}
      >
        <path
          d={
            isTeam
              ? "M540 115 V210 M180 210 H900 M180 210 V280 M540 210 V280 M900 210 V280"
              : "M85 230 H995"
          }
          stroke="#ccd7e5"
          strokeWidth="3"
          fill="none"
        />
        <path
          d={isTeam ? "M540 115 V210 M180 210 H900" : "M85 230 H995"}
          stroke="#5f99e6"
          strokeWidth="3"
          fill="none"
          pathLength="1"
          strokeDasharray="1"
          strokeDashoffset={1 - progress}
        />
      </svg>
      {items.map((item, i) => {
        const [title, desc] = item.split(" / ");
        const x = isTeam
          ? i === 0
            ? 43
            : 8 + (i - 1) * 32
          : 5 + i * (82 / (items.length - 1));
        const active = i <= Math.floor(progress * (items.length - 1));
        return (
          <div
            key={item}
            style={{
              position: "absolute",
              left: `${x}%`,
              top: isTeam ? (i === 0 ? 45 : 280) : 170,
              width: isTeam ? 205 : 145,
              transform: isTeam ? undefined : "translateX(-5%)",
              background: "white",
              border: `1.5px solid ${active ? "#70a5e9" : p.line}`,
              borderRadius: 13,
              padding: "19px 14px",
              boxShadow: active ? "0 8px 25px #40649612" : undefined,
            }}
          >
            <div
              style={{
                ...chip(active ? "#267965" : "#6c7b91"),
                padding: "5px 9px",
                fontSize: 16,
              }}
            >
              {active ? "✓" : "○"} {active ? "Ready" : "Pending"}
            </div>
            <div style={{ fontSize: 21, fontWeight: 600, marginTop: 16 }}>
              {title}
            </div>
            {desc && (
              <div
                style={{
                  fontSize: 18,
                  color: p.muted,
                  lineHeight: 1.5,
                  marginTop: 10,
                }}
              >
                {desc}
              </div>
            )}
          </div>
        );
      })}
      <div
        style={{
          position: "absolute",
          bottom: 24,
          left: 28,
          fontSize: 20,
          color: p.muted,
        }}
      >
        {phase === 0
          ? "Define the work"
          : phase === 1
            ? "Follow the execution"
            : "Inspect the result"}
      </div>
    </div>
  );
}
