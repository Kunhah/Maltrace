import { useEffect, useMemo, useRef, useState } from "react";
import type { CanonicalType, Entity } from "../types";

interface Props {
  entities: Entity[];
  canonicalTypes: CanonicalType[];
  selectedIds: string[];
  onSelect: (id: string) => void;
}

interface GeoPoint {
  entity: Entity;
  lat: number;
  lng: number;
  /// Best-effort label string for the popup. Falls back to the entity
  /// label, but Geolocation entities often carry a full address inside
  /// `data.full_data` which is what investigators want to see.
  popup: string;
}

/// Map view. Plots every entity that exposes a lat/lng coordinate pair
/// on a Leaflet map. Sources we extract from:
///
///   1. `entity.label` matching "LAT,LNG" (PHYSICAL_COORDINATES SF events
///      land here verbatim — they emit data like "39.7392,-104.9903").
///   2. `entity.data.extra.{lat,lng | latitude,longitude}` for sidecars
///      that emit structured geo (brazuca, future sidecars).
///   3. `entity.data.full_data` matching the same "LAT,LNG" pattern as
///      a fallback for older rows.
///
/// Leaflet is loaded dynamically so the desktop bundle stays small when
/// the user doesn't open the map. If Leaflet isn't installed, the view
/// degrades to a static instruction card.
export function GeoView({ entities, canonicalTypes, selectedIds, onSelect }: Props) {
  const points = useMemo(() => extractPoints(entities), [entities]);
  const colorByType = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of canonicalTypes) m.set(t.id, t.color);
    return m;
  }, [canonicalTypes]);

  const containerRef = useRef<HTMLDivElement | null>(null);
  // `LeafletAPI` is `typeof import("leaflet")` once loaded; kept loose
  // so we never import the module at the top level (the bundle for
  // users who don't open the map is unchanged).
  const leafletRef = useRef<unknown>(null);
  const mapRef = useRef<unknown>(null);
  const layerRef = useRef<unknown>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  // One-shot init: load Leaflet, build the map, attach to the container.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Leaflet is published as both ESM and CJS; the dynamic import
        // gives us a namespace with `default` on most bundlers. The
        // `@ts-ignore` is because Leaflet is an optional runtime
        // dependency — if the user hasn't run `npm install`, the
        // import throws and we fall through to the friendly error
        // card. Without the suppression, tsc would refuse to compile
        // a clean checkout.
        // @ts-ignore — optional runtime dep, see comment above
        const mod = await import(/* @vite-ignore */ "leaflet");
        // Inject Leaflet's stylesheet — needed for marker positioning.
        if (!document.querySelector('link[data-leaflet="1"]')) {
          const link = document.createElement("link");
          link.rel = "stylesheet";
          link.href =
            "https://unpkg.com/leaflet@1.9.4/dist/leaflet.css";
          link.setAttribute("data-leaflet", "1");
          link.crossOrigin = "";
          document.head.appendChild(link);
        }
        if (cancelled || !containerRef.current) return;
        // Bundlers sometimes give us `{default: L}` and sometimes the
        // namespace directly — handle both.
        const L = (mod as { default?: unknown }).default ?? mod;
        leafletRef.current = L;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const Lany = L as any;
        const map = Lany.map(containerRef.current, {
          zoomControl: true,
          attributionControl: true,
        }).setView([20, 0], 2);
        Lany.tileLayer(
          "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
          {
            maxZoom: 18,
            attribution:
              '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
          },
        ).addTo(map);
        mapRef.current = map;
        layerRef.current = Lany.layerGroup().addTo(map);
        setLoaded(true);
      } catch (e) {
        if (!cancelled) {
          setError(
            "Leaflet not installed. Run `npm install leaflet` in maltrace/app to enable the map view.",
          );
          // eslint-disable-next-line no-console
          console.error("leaflet import failed:", e);
        }
      }
    })();
    return () => {
      cancelled = true;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const map = mapRef.current as any;
      try {
        map?.remove?.();
      } catch {
        /* noop */
      }
      mapRef.current = null;
      layerRef.current = null;
    };
  }, []);

  // Diff marker layer on points / selection / colors change.
  useEffect(() => {
    if (!loaded) return;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const Lany = leafletRef.current as any;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const layer = layerRef.current as any;
    if (!Lany || !layer) return;
    layer.clearLayers();
    const selected = new Set(selectedIds);
    for (const p of points) {
      const color = colorByType.get(p.entity.entity_type) ?? "#5eead4";
      const isSelected = selected.has(p.entity.id);
      const marker = Lany.circleMarker([p.lat, p.lng], {
        radius: isSelected ? 9 : 6,
        color: isSelected ? "#5eead4" : color,
        fillColor: color,
        fillOpacity: 0.8,
        weight: isSelected ? 3 : 2,
      })
        .bindTooltip(
          `<strong>${escapeHtml(p.entity.entity_type)}</strong><br/>${escapeHtml(
            p.popup,
          )}`,
          { direction: "top", offset: [0, -8] },
        )
        .on("click", () => onSelect(p.entity.id));
      layer.addLayer(marker);
    }
    // Fit map to the markers on first paint with data — but don't keep
    // zooming when the user pans / zooms later.
    if (points.length > 0 && !layer._hasFit) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const bounds = (Lany as any).latLngBounds(
        points.map((p) => [p.lat, p.lng] as [number, number]),
      );
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (mapRef.current as any).fitBounds(bounds, {
        padding: [40, 40],
        maxZoom: 8,
      });
      layer._hasFit = true;
    }
  }, [points, selectedIds, colorByType, loaded, onSelect]);

  if (error) {
    return (
      <div className="view-empty">
        <div className="view-empty-title">Map view unavailable</div>
        <div className="view-empty-sub muted">{error}</div>
      </div>
    );
  }

  return (
    <div className="geoview">
      <div ref={containerRef} className="geoview-map" />
      <div className="geoview-legend muted small">
        {points.length === 0 ? (
          <>
            No entities with coordinates. Run a geo transform (e.g.
            <code> sfp_geoinfo</code>) on an IP to plot it here.
          </>
        ) : (
          <>{points.length} mapped {points.length === 1 ? "entity" : "entities"}</>
        )}
      </div>
    </div>
  );
}

function extractPoints(entities: Entity[]): GeoPoint[] {
  const out: GeoPoint[] = [];
  for (const e of entities) {
    const p = pointFor(e);
    if (p) out.push(p);
  }
  return out;
}

function pointFor(e: Entity): GeoPoint | null {
  // 1) Label or full_data containing "LAT,LNG".
  const fromLabel = parseLatLng(e.label);
  if (fromLabel) {
    return { entity: e, lat: fromLabel[0], lng: fromLabel[1], popup: e.label };
  }

  // Inspect the data payload for structured geo + fallback string.
  if (e.data) {
    try {
      const parsed = JSON.parse(e.data) as Record<string, unknown>;
      const extra =
        parsed.extra && typeof parsed.extra === "object"
          ? (parsed.extra as Record<string, unknown>)
          : null;

      // 2) Structured lat/lng (and {latitude, longitude} aliases).
      const numericPair = (
        a: unknown,
        b: unknown,
      ): [number, number] | null => {
        const lat = typeof a === "number" ? a : Number(a);
        const lng = typeof b === "number" ? b : Number(b);
        if (
          Number.isFinite(lat) &&
          Number.isFinite(lng) &&
          Math.abs(lat) <= 90 &&
          Math.abs(lng) <= 180
        ) {
          return [lat, lng];
        }
        return null;
      };
      if (extra) {
        const fromExtra =
          numericPair(extra.lat, extra.lng) ||
          numericPair(extra.latitude, extra.longitude);
        if (fromExtra) {
          const title =
            typeof extra.title === "string"
              ? extra.title
              : typeof extra.address === "string"
                ? extra.address
                : e.label;
          return { entity: e, lat: fromExtra[0], lng: fromExtra[1], popup: title };
        }
      }
      const fromData =
        numericPair(parsed.lat, parsed.lng) ||
        numericPair(parsed.latitude, parsed.longitude);
      if (fromData) {
        return {
          entity: e,
          lat: fromData[0],
          lng: fromData[1],
          popup: typeof parsed.full_data === "string" ? parsed.full_data : e.label,
        };
      }

      // 3) "LAT,LNG" inside full_data.
      const fullData = typeof parsed.full_data === "string" ? parsed.full_data : "";
      const fromFull = parseLatLng(fullData);
      if (fromFull) {
        return { entity: e, lat: fromFull[0], lng: fromFull[1], popup: fullData };
      }
    } catch {
      /* not JSON — fall through */
    }
  }

  return null;
}

/// Parse "LAT,LNG" (with optional whitespace, scientific notation, or a
/// surrounding "(LAT, LNG)"). Rejects anything where the values aren't
/// in the valid earth-coordinate range.
function parseLatLng(s: string | null | undefined): [number, number] | null {
  if (!s) return null;
  const trimmed = s.trim().replace(/^\(|\)$/g, "");
  const m = /^(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\s*,\s*(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)$/.exec(
    trimmed,
  );
  if (!m) return null;
  const lat = parseFloat(m[1]);
  const lng = parseFloat(m[2]);
  if (
    !Number.isFinite(lat) ||
    !Number.isFinite(lng) ||
    Math.abs(lat) > 90 ||
    Math.abs(lng) > 180
  ) {
    return null;
  }
  return [lat, lng];
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
