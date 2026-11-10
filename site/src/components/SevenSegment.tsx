// A number on a seven-segment display, the look of the show's own counters. Unlit segments
// stay faintly visible, as on the real thing.

/** Segments a-g of each digit, clockwise from the top, then the middle. */
const DIGITS = ["abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg"];

const SEGMENTS: Record<string, string> = {
  a: "M5 2h20l-4 5H9z",
  b: "M26 4v20l-4.5-3V8.5z",
  c: "M26 28v20l-4.5-4V31z",
  d: "M5 50h20l-4-5H9z",
  e: "M4 28v20l4.5-4V31z",
  f: "M4 4v20l4.5-3V8.5z",
  g: "M6 26l3-2.5h12l3 2.5-3 2.5H9z",
};

export function SevenSegment({
  value,
  digits,
  className,
}: {
  value: number;
  digits: number;
  className?: string;
}) {
  const text = String(Math.max(0, Math.floor(value))).padStart(digits, " ").slice(-digits);
  return (
    <span className={className ? `seven ${className}` : "seven"} aria-hidden="true">
      {[...text].map((char, index) => {
        const lit = char === " " ? "" : DIGITS[Number(char)];
        return (
          <svg key={index} viewBox="0 0 30 52">
            {Object.entries(SEGMENTS).map(([name, path]) => (
              <path key={name} d={path} className={lit.includes(name) ? "on" : "off"} />
            ))}
          </svg>
        );
      })}
    </span>
  );
}
