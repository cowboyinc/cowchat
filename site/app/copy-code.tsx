"use client";

import { useState } from "react";

export default function CopyCode({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      className="relative inline cursor-pointer rounded-md px-0.5 underline decoration-border-hover decoration-dotted underline-offset-4 hover:bg-surface-600 hover:decoration-border-pressed"
      type="button"
      aria-live="polite"
      aria-label={`Copy “${text}” to the clipboard`}
      title="Click to copy"
      onClick={() => {
        navigator.clipboard.writeText(text).then(
          () => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          },
          () => {
            /* clipboard denied — leave the snippet unchanged */
          },
        );
      }}
    >
      <code className="type-code-sm">{text}</code>
      {copied && (
        <span className="absolute -top-7 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-md bg-text-primary px-2 py-0.5 type-caption text-surface-500">
          Copied
        </span>
      )}
    </button>
  );
}
