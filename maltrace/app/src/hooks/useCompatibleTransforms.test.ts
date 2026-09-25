import { describe, it, expect } from "vitest";
import { renderHook } from "@testing-library/react";
import {
  useCompatibleTransforms,
  useCompatibleTransformsForMany,
} from "./useCompatibleTransforms";
import type { CanonicalType, Entity, TransformMeta } from "../types";

const DOMAIN_TYPE: CanonicalType = {
  id: "Domain",
  label: "Domain",
  kind: "asset",
  color: "#22c55e",
  sf_inputs: ["DOMAIN_NAME", "INTERNET_NAME"],
  default_sf_event: "DOMAIN_NAME",
  custom: false,
};

const EMAIL_TYPE: CanonicalType = {
  id: "EmailAddress",
  label: "Email",
  kind: "identity",
  color: "#a855f7",
  sf_inputs: ["EMAILADDR"],
  default_sf_event: "EMAILADDR",
  custom: false,
};

const TYPES = [DOMAIN_TYPE, EMAIL_TYPE];

function transform(
  id: string,
  consumes: string[],
  passivity?: "active" | "passive" | "unknown",
): TransformMeta {
  return {
    id,
    name: id,
    description: "",
    categories: [],
    group: [],
    consumes,
    produces: [],
    source: "spiderfoot",
    passivity,
  };
}

function entity(entity_type: string, source_event_type: string | null = null): Entity {
  return {
    id: `e-${entity_type}`,
    case_id: 1,
    entity_type,
    source_event_type,
    label: `${entity_type} entity`,
    data: null,
    pos_x: null,
    pos_y: null,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("useCompatibleTransforms", () => {
  const transforms: TransformMeta[] = [
    transform("sfp_whois", ["DOMAIN_NAME"], "active"),
    transform("sfp_dnsresolve", ["DOMAIN_NAME"], "active"),
    transform("sfp_googlesearch", ["DOMAIN_NAME"], "passive"),
    transform("sfp_emailrep", ["EMAILADDR"], "passive"),
    transform("sfp__stor", ["*"], "passive"),
  ];

  it("returns transforms that consume the entity's SF events", () => {
    const { result } = renderHook(() =>
      useCompatibleTransforms(entity("Domain"), TYPES, transforms, ""),
    );
    const ids = result.current.compatible.map((t) => t.id);
    // wildcard + every DOMAIN_NAME consumer
    expect(ids).toContain("sfp_whois");
    expect(ids).toContain("sfp_dnsresolve");
    expect(ids).toContain("sfp_googlesearch");
    expect(ids).toContain("sfp__stor");
    // EmailAddress-only consumers are filtered out
    expect(ids).not.toContain("sfp_emailrep");
  });

  it("returns nothing for Collection entities", () => {
    const { result } = renderHook(() =>
      useCompatibleTransforms(entity("Collection"), TYPES, transforms, ""),
    );
    expect(result.current.compatible).toEqual([]);
  });

  it("filters by substring on id and name", () => {
    const { result } = renderHook(() =>
      useCompatibleTransforms(entity("Domain"), TYPES, transforms, "whois"),
    );
    expect(result.current.compatible.map((t) => t.id)).toEqual(["sfp_whois"]);
  });

  it("stealthOnly filters out active transforms", () => {
    const { result } = renderHook(() =>
      useCompatibleTransforms(entity("Domain"), TYPES, transforms, "", true),
    );
    const ids = result.current.compatible.map((t) => t.id);
    expect(ids).toContain("sfp_googlesearch");
    expect(ids).toContain("sfp__stor");
    expect(ids).not.toContain("sfp_whois");
    expect(ids).not.toContain("sfp_dnsresolve");
  });

  it("narrows to source_event_type when set on the entity", () => {
    // INTERNET_NAME-tagged Domain — only INTERNET_NAME consumers should match.
    const e = entity("Domain", "INTERNET_NAME");
    const tForInternetName = transform("sfp_internet_name_consumer", ["INTERNET_NAME"]);
    const { result } = renderHook(() =>
      useCompatibleTransforms(
        e,
        TYPES,
        [...transforms, tForInternetName],
        "",
      ),
    );
    const ids = result.current.compatible.map((t) => t.id);
    // Wildcard still matches; INTERNET_NAME consumer matches; pure
    // DOMAIN_NAME consumers don't.
    expect(ids).toContain("sfp_internet_name_consumer");
    expect(ids).toContain("sfp__stor");
    expect(ids).not.toContain("sfp_whois");
  });
});

describe("useCompatibleTransformsForMany", () => {
  it("returns transforms compatible with every selected entity", () => {
    const transforms: TransformMeta[] = [
      transform("sfp_whois", ["DOMAIN_NAME"]),
      transform("sfp_emailrep", ["EMAILADDR"]),
      transform("any", ["*"]),
    ];
    const entities = [entity("Domain"), entity("EmailAddress")];
    const { result } = renderHook(() =>
      useCompatibleTransformsForMany(entities, TYPES, transforms, ""),
    );
    expect(result.current.compatible.map((t) => t.id)).toEqual(["any"]);
  });

  it("empty selection returns empty result", () => {
    const { result } = renderHook(() =>
      useCompatibleTransformsForMany(
        [],
        TYPES,
        [transform("any", ["*"])],
        "",
      ),
    );
    expect(result.current.compatible).toEqual([]);
  });

  it("rejects any selection containing a Collection", () => {
    const { result } = renderHook(() =>
      useCompatibleTransformsForMany(
        [entity("Domain"), entity("Collection")],
        TYPES,
        [transform("any", ["*"])],
        "",
      ),
    );
    expect(result.current.compatible).toEqual([]);
  });
});
