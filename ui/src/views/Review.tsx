import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { DesktopOnly, Empty, Segmented } from "../components/ui";
import { listIncidents } from "../lib/api";
import { ReviewRow } from "./Overview";

type Show = "open" | "decided";

export function Review() {
  const [show, setShow] = useState<Show>("open");
  const incidents = useQuery({
    queryKey: ["incidents"],
    queryFn: listIncidents,
    refetchInterval: 5000,
  });
  if (incidents.isSuccess && incidents.data === null) return <DesktopOnly />;
  const all = incidents.data ?? [];
  const shown = all.filter((incident) =>
    show === "open" ? incident.status === "open" : incident.status !== "open",
  );
  return (
    <div className="stack">
      <header className="page-header">
        <div>
          <h1>Review</h1>
          <p className="lede">
            Things that didn’t match what you asked, or that you asked to be told about.
          </p>
        </div>
        <Segmented
          label="Show"
          value={show}
          options={[
            { value: "open", label: "Needs a look" },
            { value: "decided", label: "Decided" },
          ]}
          onChange={setShow}
        />
      </header>
      {incidents.isSuccess && shown.length === 0 ? (
        <Empty title={show === "open" ? "Nothing needs a look" : "Nothing decided yet"}>
          {show === "open"
            ? "When something doesn’t match what you asked, it appears here."
            : "Items you mark as fine or not OK stay here for reference."}
        </Empty>
      ) : (
        <div className="card list">
          {shown.map((incident) => (
            <ReviewRow key={incident.id} incident={incident} />
          ))}
        </div>
      )}
    </div>
  );
}
