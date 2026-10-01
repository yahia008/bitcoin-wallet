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

export const ChevronIcon = (p: Props) => (
  <Icon {...p} className="h-4 w-4">
    <path d="M6 8l4 4 4-4" />
  </Icon>
);

export const CheckIcon = ({ className = "h-4 w-4", ...p }: Props) => (
  <Icon {...p} className={className}>
    <path d="M4.5 10.5l3.5 3.5 7.5-8" />
  </Icon>
);

export const PlusIcon = (p: Props) => (
  <Icon {...p} className="h-4 w-4">
    <path d="M10 4v12M4 10h12" />
  </Icon>
);

export const CopyIcon = (p: Props) => (
  <Icon {...p}>
    <rect x="7" y="7" width="9" height="9" rx="1.5" />
    <path d="M13 7V5.5A1.5 1.5 0 0 0 11.5 4h-6A1.5 1.5 0 0 0 4 5.5v6A1.5 1.5 0 0 0 5.5 13H7" />
  </Icon>
);

export const ExternalLinkIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M11 4h5v5M16 4l-7 7M14 11.5V15a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h3.5" />
  </Icon>
);

export const PencilIcon = (p: Props) => (
  <Icon {...p}>
    <path d="M13.5 4.5l2 2L7 15H5v-2l8.5-8.5Z" />
  </Icon>
);

export const QrIcon = (p: Props) => (
  <Icon {...p}>
    <rect x="4" y="4" width="4.5" height="4.5" rx="0.5" />
    <rect x="11.5" y="4" width="4.5" height="4.5" rx="0.5" />
    <rect x="4" y="11.5" width="4.5" height="4.5" rx="0.5" />
    <path d="M11.5 11.5h1.5v1.5M16 11.5v.01M11.5 16h.01M14.5 14.5H16V16" />
  </Icon>
);

export const PlusCircleIcon = (p: Props) => (
  <Icon {...p}>
    <circle cx="10" cy="10" r="6.5" />
    <path d="M10 7v6M7 10h6" />
  </Icon>
);
