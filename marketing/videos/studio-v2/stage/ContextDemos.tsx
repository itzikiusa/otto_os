import React from "react";
import type { Chapter } from "../model";
import { font, mono, palette as p, tween } from "../design";
import { chip, panel, Code, Rows } from "./primitives";
export function Document({ ch, phase }: { ch: Chapter; phase: number }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "245px 1fr", gap: 25 }}>
      <div>
        <div style={{ fontSize: 18, color: p.muted, marginBottom: 18 }}>
          In this workspace
        </div>
        {ch.items.map((item, i) => (
          <div
            key={item}
            style={{
              padding: "16px 16px",
              fontSize: 21,
              borderRadius: 7,
              background: i === phase ? "#e5edf8" : undefined,
              color: i === phase ? "#315f99" : p.muted,
            }}
          >
            {item.split(" / ")[0]}
          </div>
        ))}
      </div>
      <div style={{ ...panel, padding: "30px 38px", minHeight: 440 }}>
        <div style={{ fontSize: 18, color: p.muted, marginBottom: 18 }}>
          Atlas / Knowledge
        </div>
        <h2 style={{ fontSize: 36, letterSpacing: -1, margin: "0 0 24px" }}>
          {ch.heading}
        </h2>
        <p
          style={{
            fontSize: 23,
            lineHeight: 1.6,
            margin: "0 0 23px",
            color: p.muted,
          }}
        >
          A checkout should survive a transient failure without repeating the
          payment.
        </p>
        {[
          "Bound the number of attempts.",
          "Preserve the idempotency key.",
          "Record the final outcome.",
        ].map((t, i) => (
          <div
            key={t}
            style={{
              padding: "14px 16px",
              marginBottom: 10,
              borderLeft: `3px solid ${phase >= i ? "#7da9e3" : p.line}`,
              background: phase === i ? "#f0f5fc" : undefined,
              fontSize: 22,
            }}
          >
            {" "}
            {phase >= i ? "✓" : "○"} {t}
          </div>
        ))}
        <div style={{ marginTop: 24, ...chip("#4969a6") }}>
          {ch.view === "browser"
            ? "2 marks · Send to session"
            : "Linked to: Checkout reliability"}
        </div>
      </div>
    </div>
  );
}
export function BrowserDemo({ phase }: { phase: number }) {
  return (
    <div style={panel}>
      <div
        style={{
          padding: "17px 23px",
          background: "#f7f9fc",
          display: "flex",
          gap: 20,
          borderBottom: `1px solid ${p.line}`,
        }}
      >
        <span style={{ fontSize: 21, color: p.muted }}>‹ &nbsp; ›</span>
        <span style={{ fontSize: 21, flex: 1, color: "#455d7c" }}>
          https://docs.example.com/retries
        </span>
        <span style={chip("#4969a6")}>Reader</span>
        <span style={{ fontSize: 20, padding: 8, color: p.muted }}>Live</span>
      </div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 290px",
          minHeight: 400,
        }}
      >
        <div style={{ padding: 32 }}>
          <h2 style={{ fontSize: 34, margin: "0 0 22px" }}>
            Retrying a request
          </h2>
          <p style={{ fontSize: 23, lineHeight: 1.8, color: p.muted }}>
            Transient errors can recover. A retry policy needs a limit and a
            clear failure path.
          </p>
          <p
            style={{
              fontSize: 23,
              lineHeight: 1.8,
              background: phase > 0 ? "#fff0c6" : undefined,
              padding: 12,
            }}
          >
            Keep the same idempotency key across attempts to avoid duplicating
            the operation.
          </p>
          <div style={{ fontSize: 21, color: p.muted }}>
            Reference / Retry contract
          </div>
        </div>
        <div
          style={{
            borderLeft: `1px solid ${p.line}`,
            padding: 24,
            background: "#fafbfd",
          }}
        >
          <div style={{ fontSize: 23, fontWeight: 600 }}>Marks</div>
          {phase > 0 && (
            <p style={{ fontSize: 21, lineHeight: 1.6 }}>
              Check whether the retry path preserves this key.
            </p>
          )}
          <div style={{ ...chip("#4969a6"), marginTop: 30 }}>
            {phase === 2 ? "Send to session" : "Add a mark"}
          </div>
        </div>
      </div>
    </div>
  );
}
export function Chat({ ch, phase }: { ch: Chapter; phase: number }) {
  return (
    <div style={{ display: "grid", gridTemplateColumns: "280px 1fr", gap: 22 }}>
      <div>
        <div style={panel}>
          {ch.items.slice(0, 3).map((item, index) => {
            const [label, value] = item.split(' / ');
            return <div key={item} style={{padding: '23px 24px', borderBottom: index < 2 ? `1px solid ${p.line}` : undefined, background: phase === index ? '#eaf2ff' : 'white'}}>
              <div style={{fontSize: 22, fontWeight: 600, marginBottom: 9}}>{label}</div>
              <div style={{fontSize: 21, lineHeight: 1.4, color: p.muted}}>{value}</div>
            </div>;
          })}
        </div>
      </div>
      <div style={{ ...panel, padding: 26, height: 470 }}>
        <div style={{ fontSize: 22, color: p.muted, marginBottom: 25 }}>
          {ch.id === "personal"
            ? "Chat / Atlas release assistant"
            : "Thread / Checkout investigation"}
        </div>
        <div
          style={{
            marginLeft: 90,
            background: "#edf3fc",
            padding: "19px 23px",
            borderRadius: "14px 14px 3px 14px",
            fontSize: 23,
            lineHeight: 1.5,
          }}
        >
          What needs attention before the release?
        </div>
        {phase > 0 && (
          <div
            style={{
              marginTop: 26,
              marginRight: 55,
              padding: "19px 23px",
              background: "#f5f8fa",
              borderRadius: "14px 14px 14px 3px",
              fontSize: 23,
              lineHeight: 1.55,
            }}
          >
            <div style={{ fontSize: 18, color: "#527896", marginBottom: 12 }}>
              Atlas assistant
            </div>
            The retry change has a new regression test. The review is ready for
            you to inspect.
          </div>
        )}
        <div
          style={{
            marginTop: 32,
            border: `1px solid ${p.line}`,
            borderRadius: 8,
            padding: 16,
            color: p.muted,
            fontSize: 21,
          }}
        >
          {phase === 2 ? "Show me the evidence." : "Message the agent…"}
        </div>
      </div>
    </div>
  );
}
export function Mobile({ phase }: { phase: number }) {
  return (
    <div
      style={{
        height: 475,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 65,
      }}
    >
      <div
        style={{
          width: 245,
          height: 456,
          background: p.ink,
          padding: 12,
          borderRadius: 36,
          boxShadow: "0 20px 50px #15294324",
        }}
      >
        <div
          style={{
            height: "100%",
            background: "#f5f8fc",
            borderRadius: 25,
            overflow: "hidden",
          }}
        >
          <div
            style={{
              width: 82,
              height: 17,
              background: p.ink,
              borderRadius: "0 0 14px 14px",
              margin: "0 auto",
            }}
          />
          <div style={{ padding: 22 }}>
            <h3 style={{ fontSize: 25, margin: "13px 0" }}>Otto</h3>
            <div style={{ fontSize: 15, color: p.muted }}>
              Checkout investigation
            </div>
            <div style={{ ...chip(), fontSize: 15, marginTop: 18 }}>
              ● Session active
            </div>
            <div
              style={{
                marginTop: 24,
                fontFamily: mono,
                fontSize: 14,
                lineHeight: 1.85,
              }}
            >
              Reading retry.ts
              <br />
              Checking failure path
              <br />
              <br />
              The review is ready.
              <br />
              Inspect the evidence.
            </div>
          </div>
        </div>
      </div>
      <div style={{ width: 440 }}>
        <h3 style={{ fontSize: 36, letterSpacing: -1, margin: "0 0 25px" }}>
          One session.
          <br />
          The access you choose.
        </h3>
        <Rows
          items={[
            "Permission / " + (phase === 0 ? "Viewer" : "Editor"),
            "Expiry / Choose a duration",
            "Access / Revoke at any time",
          ]}
          phase={phase}
        />
      </div>
    </div>
  );
}
export function Snip({ phase }: { phase: number }) {
  return (
    <div style={{ ...panel, height: 470, padding: 35, position: "relative" }}>
      <div style={{ fontSize: 26, fontWeight: 600 }}>Checkout</div>
      <div
        style={{
          width: 550,
          margin: "45px auto",
          padding: 30,
          border: `1px solid ${p.line}`,
          borderRadius: 12,
        }}
      >
        <div style={{ fontSize: 27 }}>We could not complete your payment.</div>
        <p style={{ fontSize: 21, color: p.muted }}>Please try again.</p>
        <span style={chip("#4969a6")}>Retry checkout</span>
      </div>
      <div
        style={{
          position: "absolute",
          left: 220,
          top: 130,
          width: 690,
          height: 255,
          border: `2px ${phase === 0 ? "dashed" : "solid"} #5b95e0`,
          borderRadius: 8,
        }}
      />
      {phase > 0 && (
        <svg
          style={{ position: "absolute", left: 620, top: 220 }}
          width="230"
          height="120"
        >
          <path
            d="M205 20 Q130 110 15 95 M15 95 L40 69 M15 95 L48 112"
            fill="none"
            stroke="#c46e6e"
            strokeWidth="5"
            strokeLinecap="round"
          />
        </svg>
      )}
      <div style={{ position: "absolute", right: 30, bottom: 25, ...chip() }}>
        {phase === 2
          ? "Copied · Ready to paste"
          : phase === 1
            ? "Annotate the detail"
            : "Drag to select a region"}
      </div>
    </div>
  );
}
export function Windows({ phase }: { phase: number }) {
  return (
    <div
      style={{
        height: 470,
        position: "relative",
        background: "#dce6f0",
        borderRadius: 12,
        overflow: "hidden",
      }}
    >
      {[0, 1].map((i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            left: 25 + i * 390,
            top: 25 + i * 80,
            width: 590,
            height: 340,
            ...panel,
            boxShadow: "0 15px 30px #22355024",
            transform: `translateX(${i === 1 && phase === 0 ? 170 : 0}px)`,
          }}
        >
          <div style={{ padding: 17, background: "#f6f8fc", fontSize: 19 }}>
            ● ● ●{" "}
            <span style={{ marginLeft: 20 }}>
              Atlas / {i ? "Git review" : "Agent session"}
            </span>
          </div>
          <div
            style={{
              padding: 25,
              background: i ? "white" : p.ink,
              height: 285,
            }}
          >
            {i ? (
              <Code
                lines={[
                  "+ boundRetry(attempt);",
                  "+ preserveIdempotencyKey();",
                  "",
                  "Review / Inspect the findings",
                ]}
              />
            ) : (
              <Code
                lines={[
                  "› Investigate the timeout",
                  "",
                  "Reading the failure path…",
                  "",
                  "Ready for review.",
                ]}
                dark
              />
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
