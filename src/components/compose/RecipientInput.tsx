// Labeled recipient input for To / Cc / Bcc fields (T044, F_G4 §4.3).
// Controlled via the compose store string fields. Shows inline validation
// feedback on blur for addresses that fail the email format check.

import { useId, useRef, useState } from "react";
import { cn } from "@/lib/cn";
import { isValidEmail, parseRecipients } from "@/lib/composeValidation";

// ── Types ────────────────────────────────────────────────────────────────────

export interface RecipientInputProps {
  /** Visible label (e.g. "To", "Cc", "Bcc"). */
  label: string;
  /** Raw comma/semicolon string from the compose store. */
  value: string;
  onChange: (next: string) => void;
  /** Optional placeholder text. */
  placeholder?: string;
  /** Auto-focus this field on mount. */
  autoFocus?: boolean;
}

// ── Component ────────────────────────────────────────────────────────────────

export function RecipientInput({
  label,
  value,
  onChange,
  placeholder,
  autoFocus = false,
}: RecipientInputProps) {
  const inputId = useId();
  const inputRef = useRef<HTMLInputElement>(null);

  // Track whether any address in the field is malformed (shown on blur).
  const [hasInvalidAddress, setHasInvalidAddress] = useState(false);

  function handleBlur() {
    if (!value.trim()) {
      setHasInvalidAddress(false);
      return;
    }
    const recipients = parseRecipients(value);
    setHasInvalidAddress(recipients.some((r) => !isValidEmail(r.email)));
  }

  function handleFocus() {
    // Clear the validation marker while the user is editing.
    setHasInvalidAddress(false);
  }

  return (
    <div className="flex items-baseline gap-3 border-b border-divider px-5 py-2.5">
      {/* Label */}
      <label
        htmlFor={inputId}
        className="w-16 shrink-0 font-ui text-[10px] font-semibold uppercase tracking-widest text-p8"
      >
        {label}
      </label>

      {/* Input */}
      <input
        ref={inputRef}
        id={inputId}
        type="email"
        multiple
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={handleBlur}
        onFocus={handleFocus}
        placeholder={placeholder}
        autoComplete="off"
        spellCheck={false}
        className={cn(
          "min-w-0 flex-1 bg-transparent font-body text-sm text-p10",
          "placeholder:text-p7 focus:outline-none",
          hasInvalidAddress && "text-red",
        )}
      />

      {/* Inline validation hint */}
      {hasInvalidAddress && (
        <span
          role="alert"
          aria-live="polite"
          className="shrink-0 font-ui text-[10px] uppercase tracking-wider text-red"
        >
          Invalid address
        </span>
      )}
    </div>
  );
}
