import React from "react";
import { useCurrentFrame } from "remotion";
import type { Chapter } from "./model";
import { font, mono, palette as p, tween } from "./design";

import { chip, Code, Dot, Rows } from "./stage/primitives";
import { Terminal, Diff, Flow } from "./stage/WorkDemos";
import {
  Document,
  BrowserDemo,
  Chat,
  Mobile,
  Snip,
  Windows,
} from "./stage/ContextDemos";
import { DataDemo, Metrics, Cloud } from "./stage/DataDemos";

export function ProductStage({
  chapter: ch,
  phase,
}: {
  chapter: Chapter;
  phase: number;
}) {
  const frame = useCurrentFrame();
  let content: React.ReactNode;
  switch (ch.view) {
    case "terminal":
      content = <Terminal ch={ch} phase={phase} frame={frame} />;
      break;
    case "diff":
      content = <Diff phase={phase} />;
      break;
    case "pipeline":
    case "canvas":
    case "team":
    case "work":
      content = <Flow ch={ch} phase={phase} frame={frame} />;
      break;
    case "document":
      content = <Document ch={ch} phase={phase} />;
      break;
    case "browser":
      content = <BrowserDemo phase={phase} />;
      break;
    case "database":
    case "events":
    case "api":
    case "connections":
      content = <DataDemo ch={ch} phase={phase} />;
      break;
    case "metrics":
      content = <Metrics ch={ch} phase={phase} frame={frame} />;
      break;
    case "cloud":
    case "kubernetes":
      content = <Cloud ch={ch} phase={phase} />;
      break;
    case "chat":
      content = <Chat ch={ch} phase={phase} />;
      break;
    case "mobile":
      content = <Mobile phase={phase} />;
      break;
    case "snip":
      content = <Snip phase={phase} />;
      break;
    case "windows":
      content = <Windows phase={phase} />;
      break;
    case "logs":
      content = (
        <div
          style={{
            background: p.ink,
            padding: 30,
            borderRadius: 12,
            height: 450,
          }}
        >
          <div style={{ color: p.mint, fontSize: 21, marginBottom: 25 }}>
            ● Follow logs &nbsp; / &nbsp; checkout-api
          </div>
          <Code
            dark
            lines={ch.items.concat(
              phase > 0
                ? ["", "checkout-api-9c2 / status=200 duration=148ms"]
                : [],
            )}
          />
        </div>
      );
      break;
    default:
      content = (
        <>
          <Rows items={ch.items} phase={phase} />
          {phase === 2 && (
            <div style={{ marginTop: 25, ...chip() }}>
              {ch.view === "evidence"
                ? "Inspect the attached evidence"
                : ch.view === "benchmark"
                  ? "Compare the results"
                  : "Open the selected item to continue"}
            </div>
          )}
        </>
      );
  }
  const nav = [
    "Agents",
    "Run with Otto",
    "Mission Control",
    "Connections",
    "Git",
    "Product",
    "Vault",
    "Browser",
    "AWS",
    "Kubernetes",
    "Workflows",
  ];
  const selected: Record<string, string> = {
    agents: "Agents",
    "run-with-otto": "Run with Otto",
    git: "Git",
    proof: "Proof",
    connections: "Connections",
    database: "Connections",
    brokers: "Connections",
    api: "API",
    product: "Product",
    canvas: "Canvas",
    vault: "Vault",
    browser: "Browser",
    aws: "AWS",
    kubernetes: "Kubernetes",
    workflows: "Workflows",
    swarm: "Swarm",
    loops: "Goal Loops",
    "mission-control": "Mission Control",
    "personal-agents": "Personal Agents",
    "scheduled-tasks": "Scheduled Tasks",
    "skills-eval": "Skills Lab",
    mcp: "MCP Control Plane",
    usage: "Usage",
    settings: "Settings",
    plugins: "Plugins",
    share: "Sharing",
    snip: "Snipping Tool",
  };
  const active = selected[ch.route] ?? ch.title;
  const rail = nav.includes(active)
    ? nav
    : [...nav.slice(0, 7), active, ...nav.slice(7, 10)];
  return (
    <div
      style={{
        width: 1370,
        height: 700,
        fontFamily: font,
        color: p.ink,
        background: "#f6f8fb",
        borderRadius: 16,
        overflow: "hidden",
        border: "1px solid #bdcada",
        boxShadow: "0 35px 75px #233a5520",
        position: "relative",
      }}
    >
      <div
        style={{
          height: 49,
          display: "flex",
          alignItems: "center",
          padding: "0 22px",
          gap: 8,
          background: "#f1f4f8",
          borderBottom: "1px solid #d3dce7",
        }}
      >
        {["#da8986", "#dfc087", "#94baa5"].map((c) => (
          <Dot key={c} color={c} />
        ))}
        <span style={{ marginLeft: 23, fontSize: 18, color: "#526783" }}>
          Otto / Atlas
        </span>
        <span style={{ marginLeft: "auto", fontSize: 16, color: p.muted }}>
          Demo workspace
        </span>
      </div>
      <div style={{ display: "flex", height: 651 }}>
        <div
          style={{
            width: 180,
            flexShrink: 0,
            background: p.ink,
            padding: "25px 12px",
            color: "#a7b8cf",
          }}
        >
          <div
            style={{
              fontSize: 21,
              color: "white",
              fontWeight: 650,
              margin: "0 15px 23px",
            }}
          >
            Atlas
          </div>
          {rail.map((n, i) => (
            <div
              key={n}
              style={{
                fontSize: 17,
                padding: "11px 11px",
                marginBottom: 2,
                borderRadius: 6,
                background: n === active ? "#293e5b" : undefined,
                color: n === active ? "#edf4ff" : undefined,
                display: "flex",
                alignItems: "center",
                gap: 11,
              }}
            >
              <span style={{ fontSize: 14, opacity: 0.6 }}>
                {["◉", "▷", "◈", "⊞", "⑂", "▤", "◇"][i % 7]}
              </span>
              {n}
            </div>
          ))}
        </div>
        <div style={{ padding: "26px 32px", flex: 1, minWidth: 0 }}>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              marginBottom: 18,
            }}
          >
            <div>
              <div style={{ color: p.muted, fontSize: 17, marginBottom: 8 }}>
                {ch.detail}
              </div>
              <h2
                style={{
                  fontSize: 29,
                  letterSpacing: -0.7,
                  margin: 0,
                  fontWeight: 600,
                }}
              >
                {ch.heading}
              </h2>
            </div>
            <span style={{ ...chip("#4969a6"), maxWidth: 190, fontSize: 18 }}>
              {active}
            </span>
          </div>
          {content}
        </div>
      </div>
    </div>
  );
}
