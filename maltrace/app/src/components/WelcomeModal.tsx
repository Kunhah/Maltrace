import { useEffect, useState } from "react";

interface Props {
  onClose: () => void;
  onLoadSample: () => void | Promise<void>;
  sampleDisabled?: boolean;
}

/// First-run tour. Three short steps the user can click through or skip.
/// Reachable later via the command palette ("Show welcome tour").
export function WelcomeModal({ onClose, onLoadSample, sampleDisabled }: Props) {
  const [step, setStep] = useState(0);
  const total = 3;

  useEffect(() => {
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = "";
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowRight" && step < total - 1) setStep(step + 1);
      else if (e.key === "ArrowLeft" && step > 0) setStep(step - 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, onClose]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal welcome-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>Welcome to maltrace</h2>
          <button
            className="icon-btn"
            onClick={onClose}
            aria-label="Close welcome"
          >
            ×
          </button>
        </header>

        <div className="welcome-body">
          {step === 0 && <StepWhat />}
          {step === 1 && (
            <StepDemo
              onLoadSample={async () => {
                await onLoadSample();
                onClose();
              }}
              disabled={sampleDisabled}
            />
          )}
          {step === 2 && <StepTips />}
        </div>

        <footer className="welcome-footer">
          <div className="welcome-dots" aria-hidden="true">
            {Array.from({ length: total }).map((_, i) => (
              <span
                key={i}
                className={"welcome-dot" + (i === step ? " active" : "")}
              />
            ))}
          </div>
          <div className="welcome-nav">
            <button className="ghost-btn" onClick={onClose}>
              Skip
            </button>
            {step > 0 && (
              <button className="ghost-btn" onClick={() => setStep(step - 1)}>
                Back
              </button>
            )}
            {step < total - 1 ? (
              <button
                className="primary-btn"
                onClick={() => setStep(step + 1)}
              >
                Next →
              </button>
            ) : (
              <button className="primary-btn" onClick={onClose}>
                Get started
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}

function StepWhat() {
  return (
    <>
      <p className="welcome-lede">
        maltrace is a graph-based OSINT workbench. Drop an entity — a
        domain, email, image, person — and run <strong>transforms</strong>{" "}
        that pull related entities onto the canvas. Pivot through the
        resulting graph.
      </p>
      <ul className="welcome-list">
        <li>
          <strong>230+ transforms</strong> via the SpiderFoot adapter — WHOIS,
          DNS, breach lookups, dark-web mentions, social profiles.
        </li>
        <li>
          <strong>Face search</strong> — drop a photo, find where that face
          appears on the web.
        </li>
        <li>
          <strong>Machines</strong> — one-click investigative pipelines that
          chain transforms.
        </li>
      </ul>
    </>
  );
}

function StepDemo({
  onLoadSample,
  disabled,
}: {
  onLoadSample: () => void | Promise<void>;
  disabled?: boolean;
}) {
  return (
    <>
      <p className="welcome-lede">
        The fastest way to see how this fits together is the sample graph:
        a small WHOIS investigation built around <code>example.com</code>.
      </p>
      <button
        className="primary-btn welcome-cta"
        onClick={onLoadSample}
        disabled={disabled}
      >
        Load sample graph
      </button>
      <p className="welcome-muted">
        Or start fresh — type a domain in the topbar and click{" "}
        <span className="kbd">Add</span>. Right-click any node to see the
        transforms compatible with it.
      </p>
    </>
  );
}

function StepTips() {
  return (
    <>
      <p className="welcome-lede">A few things worth knowing up front:</p>
      <dl className="welcome-tips">
        <dt>
          <span className="kbd">Ctrl/Cmd</span>
          <span className="kbd-plus">+</span>
          <span className="kbd">K</span>
        </dt>
        <dd>
          Command palette — every action lives here. Best way to discover
          features.
        </dd>
        <dt>
          <span className="kbd">?</span>
        </dt>
        <dd>All keyboard shortcuts.</dd>
        <dt>Machines</dt>
        <dd>
          The topbar <strong>Machines</strong> button runs investigative
          pipelines: Footprint, IP Profile, Email Breach Check, Person
          Search.
        </dd>
        <dt>AI analyst</dt>
        <dd>
          Set <code>ANTHROPIC_API_KEY</code> in <code>.env</code> to enable
          the in-app Claude panel — it reads your graph for grounded
          suggestions.
        </dd>
        <dt>API keys</dt>
        <dd>
          Many SpiderFoot transforms need service keys. Add them via{" "}
          <strong>Settings</strong> in the topbar.
        </dd>
      </dl>
    </>
  );
}
