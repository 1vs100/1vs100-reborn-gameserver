import { type ReactNode, useEffect, useMemo, useState } from "react";
import { WhySection } from "./components/About";
import { GetStarted } from "./components/GetStarted";
import { LeaderboardSection } from "./components/LeaderboardSection";
import { LiveSection } from "./components/LiveSection";
import { ScheduleSection } from "./components/ScheduleSection";
import { ServerFooter } from "./components/ServerFooter";
import { Wordmark } from "./components/Wordmark";
import { DISCORD_URL, DOCS_URL, GITHUB_URL } from "./project";
import { browserTimeZone, timeZones, useTimeZone } from "./time";

const SECTIONS = [
  { id: "live", label: "Live" },
  { id: "about", label: "About" },
  { id: "schedule", label: "Schedule" },
  { id: "leaderboard", label: "Leaderboard" },
] as const;

export default function App() {
  const [zone, setZone] = useTimeZone();
  const active = useActiveSection();

  return (
    <>
      <div className="stage">
        <div className="stage-lights" aria-hidden="true" />
        <div className="stage-crowd" aria-hidden="true" />
        <header className="top-bar">
          <Wordmark />
          <nav className="nav" aria-label="Sections">
            {SECTIONS.map((section) => (
              <a
                key={section.id}
                href={`#${section.id}`}
                className={active === section.id ? "nav-item active" : "nav-item"}
                aria-current={active === section.id ? "true" : undefined}
              >
                {section.label}
              </a>
            ))}
          </nav>
          <nav className="top-links" aria-label="Project">
            <a className="top-link" href={DOCS_URL} rel="noopener">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M5 2h10l4 4v16H5zM14 2v5h5M8 11h8M8 15h8M8 19h5" style={{ fill: "none" }} stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
              </svg>
              Documentation
            </a>
            <a className="top-link" href={DISCORD_URL} rel="noopener">
              <svg viewBox="0 0 48 48" aria-hidden="true">
                <path d="M38 12a30 30 0 0 0-7.5-2.4l-1 2a28 28 0 0 0-11 0l-1-2A30 30 0 0 0 10 12C5.3 19 4 25.8 4.6 32.5A30 30 0 0 0 13.8 37l2-3a19 19 0 0 1-3-1.5l.7-.6a21 21 0 0 0 21 0l.7.6a19 19 0 0 1-3 1.5l2 3a30 30 0 0 0 9.2-4.5C44.2 24.8 42.4 18 38 12zM17.8 28.4c-1.8 0-3.3-1.7-3.3-3.8s1.5-3.8 3.3-3.8 3.3 1.7 3.3 3.8-1.5 3.8-3.3 3.8zm12.4 0c-1.8 0-3.3-1.7-3.3-3.8s1.5-3.8 3.3-3.8 3.3 1.7 3.3 3.8-1.5 3.8-3.3 3.8z" />
              </svg>
              Discord
            </a>
            <a className="top-link" href={GITHUB_URL} rel="noopener">
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M12 2a10 10 0 0 0-3.2 19.5c.5.1.7-.2.7-.5v-1.7c-2.8.6-3.4-1.2-3.4-1.2-.5-1.2-1.1-1.5-1.1-1.5-.9-.6.1-.6.1-.6 1 .1 1.6 1 1.6 1 .9 1.6 2.4 1.1 2.9.9.1-.7.4-1.1.7-1.4-2.2-.3-4.6-1.1-4.6-5 0-1.1.4-2 1-2.7-.1-.3-.4-1.3.1-2.7 0 0 .8-.3 2.8 1a9.6 9.6 0 0 1 5 0c1.9-1.3 2.8-1 2.8-1 .5 1.4.2 2.4.1 2.7.6.7 1 1.6 1 2.7 0 3.9-2.4 4.7-4.6 5 .4.3.7.9.7 1.9V21c0 .3.2.6.7.5A10 10 0 0 0 12 2z" />
              </svg>
              GitHub
            </a>
          </nav>
        </header>
        <section id="live" className="live-band" aria-labelledby="live-title">
          <div className="live-head">
            <h1 id="live-title" className="eyebrow">
              1 vs 100 is back on air
            </h1>
            <ZonePicker zone={zone} onChange={setZone} />
          </div>
          <GetStarted />
          <LiveSection zone={zone} />
        </section>
      </div>

      <main>
        <Section id="about" title="About the project">
          <WhySection />
        </Section>
        <Section id="schedule" title="Schedule">
          <ScheduleSection zone={zone} />
        </Section>
        <Section id="leaderboard" title="Leaderboard">
          <LeaderboardSection />
        </Section>
      </main>

      <ServerFooter zone={zone} />
    </>
  );
}

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section id={id} className="section" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="section-title">
        {title}
      </h2>
      {children}
    </section>
  );
}

function ZonePicker({ zone, onChange }: { zone: string; onChange: (zone: string | null) => void }) {
  const zones = useMemo(() => {
    const known = timeZones();
    return known.includes(zone) ? known : [...known, zone].sort();
  }, [zone]);
  const browser = browserTimeZone();
  return (
    <label className="zone-picker">
      <span>Times in</span>
      <select
        value={zone}
        onChange={(event) => onChange(event.target.value === browser ? null : event.target.value)}
      >
        {zones.map((name) => (
          <option key={name} value={name}>
            {name === browser ? `${name} (this device)` : name}
          </option>
        ))}
      </select>
    </label>
  );
}

/** The last section whose top has passed a line below the top bar. */
function useActiveSection(): string {
  const [active, setActive] = useState<string>(SECTIONS[0].id);
  useEffect(() => {
    const update = () => {
      const line = Math.min(window.innerHeight * 0.3, 240);
      // A short last section never reaches the line; reaching the end of the page selects it.
      const atBottom =
        window.scrollY > 0 && window.innerHeight + window.scrollY >= document.body.scrollHeight - 2;
      let current: string = SECTIONS[0].id;
      for (const section of SECTIONS) {
        const element = document.getElementById(section.id);
        if (element && element.getBoundingClientRect().top <= line) current = section.id;
      }
      setActive(atBottom ? SECTIONS[SECTIONS.length - 1].id : current);
    };
    update();
    window.addEventListener("scroll", update, { passive: true });
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update);
      window.removeEventListener("resize", update);
    };
  }, []);
  return active;
}
