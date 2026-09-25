import { describe, it, expect } from "vitest";
import { colorForTransform, detectEntityType } from "./detectEntityType";

describe("detectEntityType", () => {
  it("recognises http and https URLs", () => {
    expect(detectEntityType("https://example.com/path?q=1")).toEqual({
      type: "Url",
      label: "https://example.com/path?q=1",
    });
    expect(detectEntityType("HTTP://EXAMPLE.COM").type).toBe("Url");
  });

  it("recognises email addresses", () => {
    expect(detectEntityType("bob@example.com").type).toBe("EmailAddress");
    expect(detectEntityType("john.smith+tag@sub.example.co.uk").type).toBe(
      "EmailAddress",
    );
  });

  it("recognises IPv4 addresses but not IPv6 / MAC strings", () => {
    expect(detectEntityType("8.8.8.8").type).toBe("IpAddress");
    expect(detectEntityType("203.0.113.42").type).toBe("IpAddress");
    // IPv6 not detected — falls back to Generic / Domain depending on
    // shape. Documented limitation.
    expect(detectEntityType("2001:db8::1").type).not.toBe("IpAddress");
    expect(detectEntityType("aa:bb:cc:dd:ee:ff").type).not.toBe("IpAddress");
  });

  it("recognises domains, including subdomains", () => {
    expect(detectEntityType("example.com").type).toBe("Domain");
    expect(detectEntityType("ssh.subdomain.example.co.uk").type).toBe("Domain");
    // 1-char TLDs aren't real domains; we reject them.
    expect(detectEntityType("a.b.c").type).toBe("Generic");
  });

  it("falls back to Generic when shape is unrecognised", () => {
    expect(detectEntityType("just some text").type).toBe("Generic");
    expect(detectEntityType("3.14").type).toBe("Generic"); // looks like a number
    expect(detectEntityType("nodot").type).toBe("Generic"); // no TLD
    expect(detectEntityType("contains space.com").type).toBe("Generic");
  });

  it("trims whitespace from the label", () => {
    expect(detectEntityType("  example.com  ")).toEqual({
      type: "Domain",
      label: "example.com",
    });
  });

  it("URLs win over emails when the string looks like both", () => {
    // A pathological case — `http://foo@bar.com` is technically a URL
    // with userinfo. The URL regex should win because we check it first.
    expect(detectEntityType("http://foo@bar.com").type).toBe("Url");
  });
});

describe("colorForTransform", () => {
  it("returns a valid HSL string", () => {
    const c = colorForTransform("sfp_whois");
    expect(c).toMatch(/^hsl\(\d+, 55%, 55%\)$/);
  });

  it("is deterministic — same input maps to same output", () => {
    expect(colorForTransform("sfp_dnsresolve")).toBe(
      colorForTransform("sfp_dnsresolve"),
    );
  });

  it("different inputs map to different hues (high probability)", () => {
    const ids = [
      "sfp_whois",
      "sfp_dnsresolve",
      "sfp_haveibeenpwned",
      "sfp_virustotal",
      "face_detect",
      "brazuca_resources",
    ];
    const hues = ids.map((id) => {
      const m = colorForTransform(id).match(/^hsl\((\d+),/);
      return m ? Number(m[1]) : -1;
    });
    // All hues should be in [0, 360) and at least 4 distinct (out of 6 —
    // hash collisions are possible but rare for this small set).
    expect(new Set(hues).size).toBeGreaterThanOrEqual(4);
    for (const h of hues) {
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(360);
    }
  });
});
