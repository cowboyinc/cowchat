"use client";

import { useState } from "react";

export default function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="btn-primary-glow absolute right-3 top-3 cursor-pointer rounded-lg bg-button-primary-default px-3 py-1.5 type-body-s-strong text-button-primary-text-default hover:bg-button-primary-hover active:bg-button-primary-pressed"
      type="button"
      onClick={() => {
        navigator.clipboard.writeText(text.trim()).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? "Copied" : "Copy"}
    </button>
  );
}
