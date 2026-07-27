/**
 * HyperFrames composition engine for OpenInCut.
 * Generates HTML/CSS/GSAP compositions and renders them to video.
 */

export interface HyperFrame {
  id: string;
  name: string;
  duration: number; // ms
  width: number;
  height: number;
  elements: HyperFrameElement[];
}

export interface HyperFrameElement {
  type: "text" | "image" | "shape";
  content: string;
  x: number;
  y: number;
  width: number;
  height: number;
  style: Record<string, string>;
  animation?: {
    property: string;
    from: string;
    to: string;
    duration: number;
    delay: number;
    easing: string;
  };
}

/** Create a simple title composition */
export function createTitleComposition(
  text: string,
  options: {
    duration?: number;
    width?: number;
    height?: number;
    fontSize?: number;
    color?: string;
    bgColor?: string;
  } = {}
): HyperFrame {
  const { duration = 3000, width = 1920, height = 1080, fontSize = 72, color = "#ffffff", bgColor = "#000000" } = options;
  
  return {
    id: `frame-${Date.now()}`,
    name: `Title: ${text.slice(0, 20)}`,
    duration,
    width,
    height,
    elements: [
      {
        type: "text",
        content: text,
        x: width / 2,
        y: height / 2,
        width: width * 0.8,
        height: height * 0.3,
        style: {
          fontSize: `${fontSize}px`,
          color,
          backgroundColor: bgColor,
          textAlign: "center",
          fontFamily: "Arial, sans-serif",
        },
        animation: {
          property: "opacity",
          from: "0",
          to: "1",
          duration: 500,
          delay: 0,
          easing: "ease-out",
        },
      },
    ],
  };
}

/** Create a subtitle/lower-third composition */
export function createLowerThird(
  title: string,
  subtitle?: string,
  options: {
    duration?: number;
    width?: number;
    height?: number;
  } = {}
): HyperFrame {
  const { duration = 4000, width = 1920, height = 1080 } = options;
  
  const elements: HyperFrameElement[] = [
    {
      type: "shape",
      content: "rect",
      x: 40,
      y: height - 160,
      width: 400,
      height: 120,
      style: {
        backgroundColor: "rgba(0, 0, 0, 0.7)",
        borderRadius: "8px",
      },
      animation: {
        property: "x",
        from: "-400",
        to: "40",
        duration: 400,
        delay: 0,
        easing: "ease-out",
      },
    },
    {
      type: "text",
      content: title,
      x: 60,
      y: height - 150,
      width: 360,
      height: 50,
      style: {
        fontSize: "28px",
        color: "#ffffff",
        fontFamily: "Arial, sans-serif",
        fontWeight: "bold",
      },
      animation: {
        property: "opacity",
        from: "0",
        to: "1",
        duration: 300,
        delay: 200,
        easing: "ease-out",
      },
    },
  ];

  if (subtitle) {
    elements.push({
      type: "text",
      content: subtitle,
      x: 60,
      y: height - 100,
      width: 360,
      height: 40,
      style: {
        fontSize: "20px",
        color: "#cccccc",
        fontFamily: "Arial, sans-serif",
      },
      animation: {
        property: "opacity",
        from: "0",
        to: "1",
        duration: 300,
        delay: 400,
        easing: "ease-out",
      },
    });
  }

  return {
    id: `frame-${Date.now()}`,
    name: `Lower Third: ${title}`,
    duration,
    width,
    height,
    elements,
  };
}

/** Render composition to HTML string */
export function renderComposition(frame: HyperFrame): string {
  const elements = frame.elements.map((el) => {
    const style = Object.entries(el.style)
      .map(([k, v]) => `${k.replace(/([A-Z])/g, "-$1").toLowerCase()}: ${v}`)
      .join("; ");

    const animStyle = el.animation
      ? `animation: ${el.animation.property} ${el.animation.duration}ms ${el.animation.easing} ${el.animation.delay}ms both;`
      : "";

    return `<div style="position: absolute; left: ${el.x}px; top: ${el.y}px; width: ${el.width}px; height: ${el.height}px; ${style} ${animStyle}">${el.content}</div>`;
  }).join("\n");

  return `<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; background: #000; overflow: hidden; }
  @keyframes opacity { from { opacity: var(--from, 0); } to { opacity: var(--to, 1); } }
  @keyframes x { from { transform: translateX(var(--from, 0px)); } to { transform: translateX(var(--to, 0px)); } }
</style>
</head>
<body style="width: ${frame.width}px; height: ${frame.height}px; position: relative;">
${elements}
</body>
</html>`;
}

/** Detect if user wants a HyperFrames composition */
export function detectCompositionRequest(text: string): { type: string; args: Record<string, string> } | null {
  const lower = text.toLowerCase();
  
  if (lower.includes("create composition") || lower.includes("new composition")) {
    const titleMatch = text.match(/(?:title|text|says?)[\s:]+(.+)/i);
    return { type: "title", args: { text: titleMatch?.[1] ?? "Title" } };
  }
  
  if (lower.includes("add title") || lower.includes("title card")) {
    const titleMatch = text.match(/(?:title|card|text)[\s:]+(.+)/i);
    return { type: "title", args: { text: titleMatch?.[1] ?? "Title" } };
  }
  
  if (lower.includes("lower third") || lower.includes("name tag")) {
    const titleMatch = text.match(/(?:name|title|tag)[\s:]+(.+)/i);
    return { type: "lower_third", args: { title: titleMatch?.[1] ?? "Name" } };
  }
  
  if (lower.includes("subtitle") || lower.includes("caption")) {
    const textMatch = text.match(/(?:subtitle|caption|text)[\s:]+(.+)/i);
    return { type: "subtitle", args: { text: textMatch?.[1] ?? "Subtitle" } };
  }
  
  return null;
}
