interface Props {
  className?: string | undefined;
  /** Spoken name. Without one the icon is decorative and hidden from assistive tech. */
  label?: string | undefined;
}

/** A small speaker: this layer plays its own sound. */
export function SpeakerIcon({ className, label }: Props) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="12"
      height="12"
      className={className}
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <path d="M2 6h3l4-3v10l-4-3H2z" fill="currentColor" />
      <path
        d="M11 5.5a3.5 3.5 0 0 1 0 5M12.6 3.6a6 6 0 0 1 0 8.8"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
