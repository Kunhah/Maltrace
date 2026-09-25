/// Heuristic type-sniffer for pasted / dropped strings. Order matters —
/// URL beats email beats IP beats domain. Anything unrecognised falls
/// back to `Generic` so the user still gets *something* (they can
/// re-type via rename in the inspector).
///
/// Extracted to its own module so unit tests don't need to spin up the
/// whole App component tree.
export function detectEntityType(s: string): { type: string; label: string } {
  const trimmed = s.trim();
  // http(s)?://… → Url
  if (/^https?:\/\//i.test(trimmed)) {
    return { type: "Url", label: trimmed };
  }
  // local@domain.tld → EmailAddress
  if (
    /^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$/.test(trimmed) &&
    !trimmed.includes(" ")
  ) {
    return { type: "EmailAddress", label: trimmed };
  }
  // 1-3.1-3.1-3.1-3 → IpAddress (IPv4 only; v6 detection skipped to
  // avoid false positives on MAC addresses / hex strings).
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(trimmed)) {
    return { type: "IpAddress", label: trimmed };
  }
  // foo.bar(.baz)+ with no spaces → Domain. Conservative: at least one
  // dot + a 2-char TLD.
  if (/^[A-Za-z0-9][\w\-.]*\.[A-Za-z]{2,}$/.test(trimmed) && !trimmed.includes(" ")) {
    return { type: "Domain", label: trimmed };
  }
  return { type: "Generic", label: trimmed };
}

/// Deterministic transform-id → HSL colour. Same string in → same colour
/// out across sessions, so reopening a case shows the same edge palette
/// the investigator built up a mental model around. djb2 is overkill-
/// fast for ~250 transform ids and produces a well-spread hue range.
///
/// Extracted alongside `detectEntityType` for the same reason: testable
/// without booting React.
export function colorForTransform(id: string): string {
  let h = 5381;
  for (let i = 0; i < id.length; i++) {
    h = ((h << 5) + h + id.charCodeAt(i)) | 0;
  }
  const hue = ((h % 360) + 360) % 360;
  // Saturation + lightness fixed so colours stay readable against the
  // dark canvas. 55%/55% gives a nice "neon highlighter" tone that's
  // distinguishable from the slate-grey default edge.
  return `hsl(${hue}, 55%, 55%)`;
}
