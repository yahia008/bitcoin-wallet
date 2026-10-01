import { useId } from "react";

/** The Osok mark: an outlined Bitcoin "₿", drawn as a thick light stroke with a thinner
 * background-coloured stroke on top, so only the outline shows. Our own drawing, easy to swap. */
export function Logo({ className = "h-10 w-10" }: { className?: string }) {
  // Unique per instance, so several logos on one page don't share an SVG id.
  const id = useId();
  return (
    <svg viewBox="-2 -2 52 52" className={className} aria-hidden="true">
      <defs>
        {/* B: two bowls on a stem, plus the double strokes through the top and bottom */}
        <path
          id={id}
          d="M12 10H27a7 7 0 0 1 0 14H16M16 24H29a7 7 0 0 1 0 14H12M16 10V38M20 4V10M26 4V10M20 38V44M26 38V44"
        />
      </defs>
      <g fill="none" strokeLinejoin="miter" strokeLinecap="square">
        <use href={`#${id}`} stroke="currentColor" strokeWidth="9" />
        <use href={`#${id}`} stroke="var(--background)" strokeWidth="4" />
      </g>
    </svg>
  );
}
