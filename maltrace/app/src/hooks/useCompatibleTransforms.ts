import { useMemo } from "react";
import type { CanonicalType, Entity, TransformMeta } from "../types";

export interface CompatibleTransformsResult {
  /** Transforms that accept this entity, after filter applied, sorted by id. */
  compatible: TransformMeta[];
  /** Canonical type metadata for the entity, or null if unmapped. */
  canonical: CanonicalType | null;
  /** SpiderFoot event types the entity satisfies for transform matching. */
  acceptedSfEvents: Set<string>;
}

/**
 * Filter the transform catalogue down to those compatible with an entity.
 *
 * Match policy (in order of precision):
 *   1. If `entity.source_event_type` is set, only transforms consuming that
 *      exact SF event match — most precise.
 *   2. Otherwise, any transform whose `consumes` intersects the canonical
 *      type's `sf_inputs` matches.
 *   3. Transforms that consume `"*"` always match (SpiderFoot's "any event"
 *      wildcard — currently sfp__stor_*).
 *
 * Collections are opaque — they aren't a real OSINT type; the only sensible
 * action is Expand, which the UI surfaces separately.
 *
 * Shared between the side inspector panel and the right-click context menu
 * so the two cannot drift.
 */
export function useCompatibleTransforms(
  entity: Entity | null,
  canonicalTypes: CanonicalType[],
  transforms: TransformMeta[],
  filter: string,
  /// When true, drop every transform that isn't classified as `passive`.
  /// Backstops stealth investigations against an accidental active probe.
  stealthOnly: boolean = false,
): CompatibleTransformsResult {
  const canonical = useMemo(
    () => canonicalTypes.find((c) => c.id === entity?.entity_type) ?? null,
    [canonicalTypes, entity],
  );
  const acceptedSfEvents = useMemo(() => {
    if (!entity) return new Set<string>();
    if (entity.source_event_type) return new Set([entity.source_event_type]);
    return new Set(canonical?.sf_inputs ?? []);
  }, [entity, canonical]);
  const compatible = useMemo(() => {
    if (!entity) return [];
    if (entity.entity_type === "Collection") return [];
    const f = filter.trim().toLowerCase();
    return transforms
      .filter((t) =>
        t.consumes.some((c) => c === "*" || acceptedSfEvents.has(c)),
      )
      .filter((t) => !stealthOnly || t.passivity === "passive")
      .filter(
        (t) =>
          !f ||
          t.id.toLowerCase().includes(f) ||
          t.name.toLowerCase().includes(f),
      )
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [entity, transforms, filter, acceptedSfEvents, stealthOnly]);
  return { compatible, canonical, acceptedSfEvents };
}

/**
 * Multi-entity variant: returns transforms compatible with *every* selected
 * entity (set intersection). Empty selection → empty result. A selection
 * containing any Collection is also rejected — expand it first.
 */
export function useCompatibleTransformsForMany(
  entities: Entity[],
  canonicalTypes: CanonicalType[],
  transforms: TransformMeta[],
  filter: string,
  stealthOnly: boolean = false,
): { compatible: TransformMeta[] } {
  const compatible = useMemo(() => {
    if (entities.length === 0) return [];
    if (entities.some((e) => e.entity_type === "Collection")) return [];
    const f = filter.trim().toLowerCase();
    const perEntityEvents = entities.map((e) => {
      if (e.source_event_type) return new Set([e.source_event_type]);
      const c = canonicalTypes.find((ct) => ct.id === e.entity_type);
      return new Set(c?.sf_inputs ?? []);
    });
    return transforms
      .filter((t) =>
        perEntityEvents.every((events) =>
          t.consumes.some((c) => c === "*" || events.has(c)),
        ),
      )
      .filter((t) => !stealthOnly || t.passivity === "passive")
      .filter(
        (t) =>
          !f ||
          t.id.toLowerCase().includes(f) ||
          t.name.toLowerCase().includes(f),
      )
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [entities, canonicalTypes, transforms, filter, stealthOnly]);
  return { compatible };
}
