export type CaseView = "graph" | "map" | "timeline";

interface Props {
  current: CaseView;
  onChange: (view: CaseView) => void;
}

/// Three-way tab strip docked above the canvas. Lets the investigator
/// switch between the graph, a geo map, and a temporal timeline of the
/// same case without losing selection or run state.
export function ViewSwitcher({ current, onChange }: Props) {
  const tabs: Array<{ id: CaseView; label: string; hint: string }> = [
    { id: "graph", label: "Graph", hint: "Nodes + edges (default)" },
    { id: "map", label: "Map", hint: "Geo-locatable entities on a world map" },
    {
      id: "timeline",
      label: "Timeline",
      hint: "Entities + runs on a time axis",
    },
  ];
  return (
    <div className="view-switcher" role="tablist" aria-label="Case view">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={current === t.id}
          className={`view-tab${current === t.id ? " is-active" : ""}`}
          onClick={() => onChange(t.id)}
          title={t.hint}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}
