// Small line icons for the dashboard, drawn on a 20×20 grid in the current text colour.

type Props = { className?: string };

function Icon({ className = "h-5 w-5", children }: Props & { children: React.ReactNode }) {
  return (
    <svg
      viewBox="0 0 20 20"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {children}
    </svg>
  );
}

export const ArrowDownIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M10 4v12M5 11l5 5 5-5" />
  </Icon>
);

export const ArrowUpIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M10 16V4M5 9l5-5 5 5" />
  </Icon>
);

export const RefreshIcon = ({ spinning, ...p }: Props & { spinning?: boolean }) => (
  <Icon {...p} className={`h-5 w-5 ${spinning ? "animate-spin" : ""}`}>
    <path d="M16 10a6 6 0 1 1-1.8-4.3M16 3.5v3.5h-3.5" />
  </Icon>
);

export const DotsIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M5 10h.01M10 10h.01M15 10h.01" strokeWidth="2.6" />
  </Icon>
);

export const BackIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M12.5 4.5 7 10l5.5 5.5" />
  </Icon>
);

export const CloseIcon = (p: Props) => (
  <Icon {...p} className="h-4 w-4">
    <path d="M5 5l10 10M15 5 5 15" />
  </Icon>
);
