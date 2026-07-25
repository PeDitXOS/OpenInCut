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

export function ToastContainer() {
  const toast = useToast();
  if (!toast) return null;

  const colors: Record<string, string> = {
    success: "bg-green-600/90 text-white",
    error: "bg-red-600/90 text-white",
    info: "bg-bg2/90 text-ink border border-line",
  };

  return (
    <div className="fixed top-3 right-3 z-[9999] animate-[fadeIn_0.15s_ease-out]">
      <div className={`rounded-md px-3 py-2 text-[12px] shadow-lg ${colors[toast.type] ?? colors.info}`}>
        {toast.msg}
      </div>
    </div>
  );
}