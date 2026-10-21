import { useQuery } from "@tanstack/react-query";
import { getRuntimeInfo } from "./lib/runtime";

export function App() {
  // The query is the single typed boundary between the renderer and Rust shell.
  const runtime = useQuery({ queryKey: ["runtime"], queryFn: getRuntimeInfo });
  let runtimeHeading = "Checking runtime";
  let runtimeDetail = "Waiting for the desktop runtime response.";

  if (runtime.isError) {
    runtimeHeading = "Core connection unavailable";
    runtimeDetail = "The runtime check failed. Retry or inspect the development console.";
  } else if (runtime.data) {
    runtimeHeading = "Desktop core connected";
    runtimeDetail = `v${runtime.data.version} / ${runtime.data.platform} / ${runtime.data.architecture}`;
  } else if (!runtime.isPending) {
    runtimeHeading = "Browser preview";
    runtimeDetail = "Launch the desktop app to check the Rust core connection.";
  }

  return (
    <div className="shell">
      <header className="header">
        <a className="brand" href="#/" aria-label="Picket home">
          <span className="brand-mark" aria-hidden="true">
            p
          </span>
          Picket
        </a>
        <span className="stage">Development foundation</span>
      </header>
      <main>
        <p className="eyebrow">An independent record</p>
        <h1>
          Your agents act.
          <br />
          Picket keeps watch.
        </h1>
        <p className="intro">
          A private, local record of what you asked, what your agent claimed, and what actually
          changed.
        </p>
        <section className="layers" aria-label="The three layers of an agent action">
          <article>
            <span className="number">01</span>
            <h2>Request</h2>
            <p>What you asked your agent to do.</p>
          </article>
          <article>
            <span className="number">02</span>
            <h2>Claim</h2>
            <p>What your agent said it did.</p>
          </article>
          <article>
            <span className="number">03</span>
            <h2>Effect</h2>
            <p>What changed in your account.</p>
          </article>
        </section>
        {/* This panel proves the UI-to-Rust boundary before monitoring exists. */}
        <section className="runtime" aria-labelledby="runtime-heading">
          <div>
            <p className="eyebrow">Workspace status</p>
            <h2 id="runtime-heading">{runtimeHeading}</h2>
            <p role="status">{runtimeDetail}</p>
          </div>
          <button
            type="button"
            onClick={() => void runtime.refetch()}
            disabled={runtime.isFetching}
          >
            Check connection
          </button>
        </section>
        {/* Keep the foundation explicit about capabilities that do not exist yet. */}
        <p className="disclosure">
          Monitoring is not implemented yet. No accounts are connected and no personal data is
          collected.
        </p>
      </main>
      <footer>
        <span>Built to stay on your Mac.</span>
        <span>Local first. Read only. Independent.</span>
      </footer>
    </div>
  );
}
