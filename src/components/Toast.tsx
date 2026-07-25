
import { useEffect, useState } from "react";

let toastFn: ((msg: string, type?: "success" | "error" | "info") => void) | null = null;

export function showToast(msg: string, type: "success" | "error" | "info" = "info") {
  toastFn?.(msg, type);
}

export function useToast() {
  const [toast, setToast] = useState<{ msg: string; type: string } | null>(null);

  useEffect(() => {
    toastFn = (msg, type) => {
      setToast({ msg, type: type ?? "info" });
      setTimeout(() => setToast(null), 3000);
    };
    return () => { toastFn = null; };
  }, []);

  return toast;
}
