/**
 * Minimal FCP XML (Final Cut Pro) parser.
 *
 * Extracts clip references and timeline structure from .fcpxml / .fcpxmld
 * files so OpenInCut can import FCP projects. Handles:
 * - `<asset-clip>` → media path + in/out
 * - `<clip>` → compound clip (recursive)
 * - `<spine>` → timeline ordering
 * - `<asset>` → media file references in `<resources>`
 */

export interface FcpClip {
  /** Display name. */
  name: string;
  /** Resolved media file path (absolute or relative to the FCP XML). */
  srcPath: string;
  /** Source in-point (FCP units: seconds × duration attribute). */
  srcInUs: number;
  /** Source out-point. */
  srcOutUs: number;
  /** Timeline offset (µs). */
  offsetUs: number;
  /** Duration on timeline (µs). */
  durationUs: number;
}

export interface FcpTimeline {
  name: string;
  clips: FcpClip[];
  /** Sequence duration in µs (0 = unknown). */
  durationUs: number;
}

/** Parse FCP time string: "12345/30000s" → seconds. */
function parseFcpTime(val: string | null | undefined): number {
  if (!val) return 0;
  // FCP uses rational: "frames/30000s" or plain "12.345s"
  const m = val.match(/^(-?\d+(?:\.\d+)?)\s*\/\s*(\d+(?:\.\d+)?)s$/);
  if (m) return parseFloat(m[1]) / parseFloat(m[2]);
  const m2 = val.match(/^(-?\d+(?:\.\d+)?)s$/);
  if (m2) return parseFloat(m2[1]);
  return 0;
}

/** µs from FCP time string. */
function fcpToUs(val: string | null | undefined): number {
  return Math.round(parseFcpTime(val) * 1_000_000);
}

/**
 * Parse an FCP XML string and return extracted timelines + media paths.
 *
 * `xmlDir` is the directory containing the .fcpxml file — used to resolve
 * relative paths in `<media-rep>` src attributes.
 */
export function parseFcpXml(
  xmlText: string,
  xmlDir: string,
): { timelines: FcpTimeline[]; mediaPaths: string[] } {
  const doc = new DOMParser().parseFromString(xmlText, "text/xml");
  const parseError = doc.querySelector("parsererror");
  if (parseError) {
    throw new Error(`FCP XML parse error: ${parseError.textContent?.slice(0, 200)}`);
  }

  // Build resource map: resource-id → file path
  const resourceMap = new Map<string, string>();
  doc.querySelectorAll("resource").forEach((r) => {
    const id = r.getAttribute("id");
    const rep = r.querySelector("media-rep");
    if (id && rep) {
      const src = rep.getAttribute("src") ?? "";
      // src is usually "file:///path/to/file.mp4" or relative path
      const cleanPath = src
        .replace(/^file:\/{2,3}/, "")
        .replace(/^[A-Z]:/i, (m) => m); // preserve Windows drive letter
      resourceMap.set(id, cleanPath);
    }
  });

  const mediaPaths = new Set<string>();
  const timelines: FcpTimeline[] = [];

  // Find all sequences / projects
  const sequences = doc.querySelectorAll("sequence, project");
  sequences.forEach((seq) => {
    const name =
      seq.getAttribute("name") ??
      seq.querySelector("name")?.textContent ??
      "Untitled";
    const durationUs = fcpToUs(seq.getAttribute("duration"));
    const clips: FcpClip[] = [];

    // Walk spine elements
    const spines = seq.querySelectorAll("spine");
    spines.forEach((spine) => {
      collectClips(spine, 0, resourceMap, clips, mediaPaths, xmlDir);
    });

    // If no spines found, try direct children
    if (spines.length === 0) {
      collectClips(seq, 0, resourceMap, clips, mediaPaths, xmlDir);
    }

    if (clips.length > 0) {
      timelines.push({ name, clips, durationUs });
    }
  });

  return { timelines, mediaPaths: [...mediaPaths] };
}

function collectClips(
  parent: Element,
  baseOffsetUs: number,
  resourceMap: Map<string, string>,
  out: FcpClip[],
  mediaPaths: Set<string>,
  xmlDir: string,
) {
  for (const child of Array.from(parent.children)) {
    const tag = child.tagName.toLowerCase();

    if (tag === "asset-clip" || tag === "clip" || tag === "ref-clip") {
      const name = child.getAttribute("name") ?? "Untitled";
      const offsetUs = baseOffsetUs + fcpToUs(child.getAttribute("offset"));
      const durationUs = fcpToUs(child.getAttribute("duration"));

      // Source range
      const srcIn = fcpToUs(child.getAttribute("in"));
      const srcOut =
        srcIn + (durationUs > 0 ? durationUs : fcpToUs(child.getAttribute("duration")));

      // Resolve media path from asset reference
      const ref = child.getAttribute("ref") ?? "";
      let srcPath = resourceMap.get(ref) ?? "";

      // Make relative paths absolute against the XML directory
      if (srcPath && !srcPath.match(/^[A-Z]:\\/i) && !srcPath.startsWith("/")) {
        srcPath = xmlDir + "\\" + srcPath.replace(/\//g, "\\");
      }

      if (srcPath) {
        mediaPaths.add(srcPath);
        out.push({
          name,
          srcPath,
          srcInUs: srcIn,
          srcOutUs: srcOut,
          offsetUs,
          durationUs: durationUs > 0 ? durationUs : srcOut - srcIn,
        });
      }
    }

    // Recurse into nested spines / compound clips
    if (tag === "spine" || tag === "sequence" || tag === "project") {
      collectClips(child, baseOffsetUs, resourceMap, out, mediaPaths, xmlDir);
    }
  }
}
