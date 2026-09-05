import { Easing, interpolate } from "remotion";
export const palette = {
  ink: "#101b30",
  paper: "#edf2f6",
  white: "#ffffff",
  muted: "#6c7b91",
  line: "#dce3ec",
  blue: "#72a9ff",
  mint: "#72d4be",
  amber: "#efbd76",
};
export const font = "Instrument Sans, sans-serif";
export const mono = "SFMono-Regular, Menlo, monospace";
export const tween = (f: number, start: number, end: number, a = 0, b = 1) =>
  interpolate(f, [start, end], [a, b], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.22, 1, 0.36, 1),
  });
