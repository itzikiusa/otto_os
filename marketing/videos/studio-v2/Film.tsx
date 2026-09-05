import React, { useEffect, useState } from "react";
import {
  AbsoluteFill,
  Audio,
  Sequence,
  continueRender,
  delayRender,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { beatStarts, type Chapter, type Episode } from "./model";
import { font, palette as p, tween } from "./design";
import { ProductStage } from "./ProductStage";

function Aperture({
  accent,
  small = false,
}: {
  accent: string;
  small?: boolean;
}) {
  const frame = useCurrentFrame();
  return (
    <svg
      width={small ? 80 : 850}
      height={small ? 80 : 850}
      viewBox="0 0 850 850"
      style={{ overflow: "visible" }}
    >
      <defs>
        <linearGradient id="rim" x1="0" y1="0" x2="1" y2="1">
          <stop stopColor="#f4f9ff" />
          <stop offset=".32" stopColor={accent} />
          <stop offset=".62" stopColor="#2a4262" />
          <stop offset="1" stopColor="#b8d4f3" />
        </linearGradient>
        <radialGradient id="core">
          <stop stopColor="#203c5b" />
          <stop offset="1" stopColor="#101b30" />
        </radialGradient>
      </defs>
      <g
        transform={`translate(425 425) rotate(${small ? 0 : -24 + tween(frame, 0, 180, 0, 14)})`}
      >
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <ellipse
            key={i}
            rx={320 - i * 19}
            ry={320 - i * 19}
            fill={i === 5 ? "url(#core)" : "none"}
            stroke="url(#rim)"
            strokeWidth={i === 0 ? 3 : i === 5 ? 20 : 1.3}
            opacity={i === 5 ? 1 : 0.55 + i * 0.06}
            transform={`rotate(${i * 9}) scale(1 ${0.83 + i * 0.028})`}
          />
        ))}
        <ellipse
          rx="198"
          ry="198"
          fill="none"
          stroke="#070f1c"
          strokeWidth="48"
        />
        <path
          d="M-148 0 a148 148 0 1 1 296 0 a148 148 0 1 1-296 0 M-56-48 L-8 0 L-56 48 M26 58 H83"
          fill="none"
          stroke="url(#rim)"
          strokeWidth="33"
          strokeLinecap="round"
        />
      </g>
    </svg>
  );
}
function Titles({
  episode,
  closing = false,
}: {
  episode: Episode;
  closing?: boolean;
}) {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const duration =
    (closing ? episode.outroSeconds : episode.introSeconds) * fps;
  const enter = tween(f, 0, 40);
  const leave = tween(f, duration - 14, duration);
  return (
    <AbsoluteFill
      style={{
        background: p.ink,
        color: p.paper,
        overflow: "hidden",
        opacity: 1 - leave,
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: `radial-gradient(ellipse at 76% 42%, ${episode.accent}18, transparent 60%)`,
        }}
      />
      <div
        style={{
          position: "absolute",
          right: -80,
          top: 80,
          opacity: enter,
          transform: `translateX(${(1 - enter) * 80}px) rotate(${closing ? -12 : 0}deg)`,
        }}
      >
        <Aperture accent={episode.accent} />
      </div>
      <div
        style={{
          position: "absolute",
          left: 100,
          top: 84,
          display: "flex",
          alignItems: "center",
          gap: 15,
          fontSize: 28,
          fontWeight: 600,
        }}
      >
        <span
          style={{
            width: 14,
            height: 14,
            background: episode.accent,
            borderRadius: "50%",
          }}
        />{" "}
        Otto{" "}
        <span style={{ fontWeight: 400, color: "#93a7c1", marginLeft: 22 }}>
          Walkthroughs
        </span>
      </div>
      <div
        style={{
          position: "absolute",
          left: 100,
          top: 280,
          width: 1090,
          transform: `translateY(${(1 - enter) * 30}px)`,
          opacity: enter,
        }}
      >
        <div style={{ fontSize: 25, color: episode.accent, marginBottom: 30 }}>
          {closing
            ? "Your next step"
            : `Film ${String(episode.number).padStart(2, "0")} / ${episode.chapters.length} chapters`}
        </div>
        <h1
          style={{
            fontSize: 112,
            fontWeight: 540,
            letterSpacing: -6,
            lineHeight: 1.035,
            margin: 0,
            maxWidth: 1000,
          }}
        >
          {closing ? "Make it your workflow." : episode.title}
        </h1>
        <p
          style={{
            fontSize: 30,
            lineHeight: 1.5,
            color: "#9fafaF",
            maxWidth: 760,
            marginTop: 35,
          }}
        >
          {closing
            ? `Open ${episode.chapters[0].title === "Start a session" ? "Agents" : episode.chapters[0].detail.split(" / ")[0]} in Otto to begin. Use the chapters to revisit a step.`
            : episode.desc}
        </p>
      </div>
      <div
        style={{
          position: "absolute",
          left: 100,
          right: 100,
          bottom: 85,
          display: "flex",
          gap: 35,
          paddingTop: 25,
          borderTop: "1px solid #38506c",
        }}
      >
        {episode.chapters.map((ch, i) => (
          <div
            key={ch.id}
            style={{
              flex: 1,
              color: i === 0 ? episode.accent : "#9bacc2",
              fontSize: 21,
              lineHeight: 1.45,
            }}
          >
            <span
              style={{
                display: "block",
                fontSize: 16,
                marginBottom: 9,
                color: "#7b91ad",
              }}
            >
              {String(i + 1).padStart(2, "0")}
            </span>
            {ch.title}
          </div>
        ))}
      </div>
    </AbsoluteFill>
  );
}
function ChapterScene({
  episode,
  chapter,
  index,
}: {
  episode: Episode;
  chapter: Chapter;
  index: number;
}) {
  const f = useCurrentFrame();
  const { fps } = useVideoConfig();
  const starts = beatStarts(chapter);
  const seconds = f / fps;
  const phase = Math.max(
    0,
    starts.reduce((last, t, i) => (seconds >= t ? i : last), -1),
  );
  const enter = tween(f, 0, 22);
  const exit = tween(f, chapter.duration * fps - 12, chapter.duration * fps);
  const clickFrame = (starts[phase] + 0.35) * fps;
  const click = tween(f, clickFrame, clickFrame + 18);
  const cam = tween(f, 0, chapter.duration * fps, 1, 1.012);
  const caption = seconds >= starts[0] ? chapter.steps[phase] : "";
  return (
    <AbsoluteFill
      style={{
        background: p.paper,
        color: p.ink,
        opacity: 1 - exit,
        overflow: "hidden",
      }}
    >
      <div
        style={{
          position: "absolute",
          left: 65,
          top: 47,
          display: "flex",
          gap: 15,
          alignItems: "center",
          fontSize: 23,
          fontWeight: 600,
        }}
      >
        <span
          style={{
            width: 11,
            height: 11,
            borderRadius: "50%",
            background: p.ink,
          }}
        />{" "}
        Otto{" "}
        <span style={{ fontWeight: 400, color: p.muted, marginLeft: 18 }}>
          {episode.title}
        </span>
      </div>
      <div
        style={{
          position: "absolute",
          right: 66,
          top: 50,
          fontSize: 20,
          color: p.muted,
        }}
      >
        Chapter {index + 1} / {episode.chapters.length}
      </div>
      <div
        style={{
          position: "absolute",
          left: 65,
          top: 210,
          width: 345,
          opacity: enter,
          transform: `translateY(${(1 - enter) * 16}px)`,
        }}
      >
        <div
          style={{
            width: 50,
            height: 4,
            background: "#527bad",
            marginBottom: 28,
          }}
        />
        <h2
          style={{
            fontSize: 57,
            lineHeight: 1.07,
            fontWeight: 550,
            letterSpacing: -2.8,
            margin: 0,
          }}
        >
          {chapter.title}
        </h2>
        <div style={{ marginTop: 48 }}>
          {chapter.steps.map((step, i) => (
            <div
              key={i}
              style={{
                display: "flex",
                gap: 13,
                alignItems: "center",
                marginBottom: 22,
                opacity: phase === i ? 1 : 0.4,
              }}
            >
              <span
                style={{
                  width: 29,
                  height: 29,
                  display: "grid",
                  placeItems: "center",
                  borderRadius: "50%",
                  fontSize: 16,
                  background: phase === i ? p.ink : "transparent",
                  border: `1px solid ${p.ink}`,
                  color: phase === i ? "white" : p.ink,
                  flexShrink: 0,
                }}
              >
                {i < phase ? "✓" : i + 1}
              </span>
              <span
                style={{ fontSize: 23, fontWeight: phase === i ? 600 : 450 }}
              >
                {chapter.actions[i]}
              </span>
            </div>
          ))}
        </div>
        <div
          style={{
            marginTop: 50,
            fontSize: 18,
            color: p.muted,
            lineHeight: 1.6,
          }}
        >
          Illustrated product demonstration
          <br />
          Fictional Atlas workspace
        </div>
      </div>
      <div
        style={{
          position: "absolute",
          left: 470,
          top: 150,
          transform: `translateY(${(1 - enter) * 25}px) scale(${cam})`,
          transformOrigin: "50% 50%",
          opacity: enter,
        }}
      >
        <ProductStage chapter={chapter} phase={phase} />
        {seconds > starts[0] && (
          <div
            style={{
              position: "absolute",
              left: phase === 0 ? 360 : phase === 1 ? 730 : 1050,
              top: phase === 0 ? 200 : phase === 1 ? 340 : 450,
              transform: `translate(${tween(f, clickFrame - 14, clickFrame, 40, 0)}px, ${tween(f, clickFrame - 14, clickFrame, 30, 0)}px)`,
              opacity: seconds < starts[phase] + 2 ? 1 : 0,
            }}
          >
            <div
              style={{
                position: "absolute",
                left: -16,
                top: -16,
                width: 44,
                height: 44,
                border: "2px solid #5488ce",
                borderRadius: "50%",
                opacity: 1 - click,
                transform: `scale(${1 + click * 1.3})`,
              }}
            />
            <svg width="28" height="35" viewBox="0 0 28 35">
              <path
                d="M2 2L23 21H13L8 31Z"
                fill="#17273f"
                stroke="white"
                strokeWidth="2"
              />
            </svg>
          </div>
        )}
      </div>
      <div
        style={{
          position: "absolute",
          left: 475,
          right: 88,
          bottom: 90,
          minHeight: 82,
          display: "flex",
          alignItems: "center",
          fontSize: 30,
          lineHeight: 1.4,
          fontWeight: 480,
        }}
      >
        {caption}
      </div>
      <div
        style={{
          position: "absolute",
          left: 65,
          bottom: 43,
          fontSize: 17,
          color: p.muted,
        }}
      >
        Otto / A complete workflow
      </div>
      <div
        style={{
          position: "absolute",
          bottom: 42,
          left: 475,
          right: 82,
          display: "flex",
          gap: 7,
        }}
      >
        {episode.chapters.map((ch, i) => (
          <div
            key={ch.id}
            style={{ height: 3, flex: ch.duration, background: "#d3dde8" }}
          >
            <div
              style={{
                height: "100%",
                width: `${i < index ? 100 : i === index ? (seconds / chapter.duration) * 100 : 0}%`,
                background: "#527bad",
              }}
            />
          </div>
        ))}
      </div>
    </AbsoluteFill>
  );
}
export function Film({ episode }: { episode: Episode }) {
  const [handle] = useState(() => delayRender("Load the film typeface"));
  useEffect(() => {
    const face = new FontFace(
      "Instrument Sans",
      `url(${staticFile("studio-v2/fonts/InstrumentSans.ttf")})`,
      { weight: "100 900" },
    );
    face
      .load()
      .then((loaded) => {
        document.fonts.add(loaded);
        continueRender(handle);
      })
      .catch((err) => {
        console.error(err);
        continueRender(handle);
      });
  }, [handle]);
  return (
    <AbsoluteFill style={{ fontFamily: font, background: p.ink }}>
      <Audio src={staticFile(`studio-v2/audio/${episode.id}.wav`)} />
      <Sequence durationInFrames={episode.introSeconds * 30}>
        <Titles episode={episode} />
      </Sequence>
      {episode.chapters.map((chapter, index) => (
        <Sequence
          key={chapter.id}
          from={Math.round(chapter.start * 30)}
          durationInFrames={Math.round(chapter.duration * 30)}
          name={chapter.title}
        >
          <ChapterScene episode={episode} chapter={chapter} index={index} />
        </Sequence>
      ))}
      <Sequence
        from={Math.round((episode.duration - episode.outroSeconds) * 30)}
      >
        <Titles episode={episode} closing />
      </Sequence>
    </AbsoluteFill>
  );
}
