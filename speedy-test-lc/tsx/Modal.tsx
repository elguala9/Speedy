import React, { useEffect, useRef } from "react";

interface ModalProps {
  isOpen:    boolean;
  onClose:   () => void;
  title?:    string;
  children:  React.ReactNode;
  size?:     "sm" | "md" | "lg" | "xl";
}

const SIZE_CLASSES: Record<NonNullable<ModalProps["size"]>, string> = {
  sm: "max-w-sm",
  md: "max-w-md",
  lg: "max-w-lg",
  xl: "max-w-xl",
};

export function Modal({ isOpen, onClose, title, children, size = "md" }: ModalProps) {
  const overlayRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <div
      ref={overlayRef}
      role="dialog"
      aria-modal
      aria-labelledby={title ? "modal-title" : undefined}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={(e) => { if (e.target === overlayRef.current) onClose(); }}
    >
      <div className={`relative w-full ${SIZE_CLASSES[size]} rounded-xl bg-white shadow-xl`}>
        {title && (
          <div className="flex items-center justify-between border-b px-6 py-4">
            <h2 id="modal-title" className="text-lg font-semibold">{title}</h2>
            <button onClick={onClose} className="text-gray-400 hover:text-gray-600" aria-label="Close">✕</button>
          </div>
        )}
        <div className="px-6 py-4">{children}</div>
      </div>
    </div>
  );
}
