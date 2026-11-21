import type { ReactNode } from "react";

export function RunlarkMarkSvg({ size = 18 }: { readonly size?: number }): ReactNode {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="3" width="18" height="18" rx="4" stroke="currentColor" strokeWidth="1.75" />
      <path
        d="M7.5 8.5H16.5M7.5 12.5H13.5M7.5 16.5H11"
        stroke="#EDB830"
        strokeWidth="1.75"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function RunlarkLockupSvg(): ReactNode {
  return (
    <span className="rl-rail-brand">
      <RunlarkMarkSvg size={22} />
      <span
        style={{ fontWeight: 800, fontSize: "1rem", letterSpacing: "-0.02em", color: "#FFFFFF" }}
      >
        Runlark
      </span>
    </span>
  );
}

export function BoardIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <rect
        x="2"
        y="2.5"
        width="3.2"
        height="11"
        rx="0.8"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <rect
        x="6.4"
        y="2.5"
        width="3.2"
        height="8"
        rx="0.8"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <rect
        x="10.8"
        y="2.5"
        width="3.2"
        height="9.5"
        rx="0.8"
        stroke="currentColor"
        strokeWidth="1.4"
      />
    </svg>
  );
}

export function DecisionsIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M3 8.2L6.3 11.4L13 4.6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function ProjectsIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M2 4.5C2 3.67 2.67 3 3.5 3H6.4L7.7 4.5H12.5C13.33 4.5 14 5.17 14 6V11.5C14 12.33 13.33 13 12.5 13H3.5C2.67 13 2 12.33 2 11.5V4.5Z"
        stroke="currentColor"
        strokeWidth="1.4"
      />
    </svg>
  );
}

export function PluginsIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <rect
        x="2.5"
        y="2.5"
        width="4.5"
        height="4.5"
        rx="1"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <rect x="2.5" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M11.25 9V13.5M9 11.25H13.5"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function MenuIconSvg(): ReactNode {
  return (
    <svg
      width={18}
      height={18}
      viewBox="0 0 18 18"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M3 5H15M3 9H15M3 13H15"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function CloseIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M4 4L12 12M12 4L4 12"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function DragGripSvg(): ReactNode {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 16 16"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="5.5" cy="4" r="1.1" />
      <circle cx="10.5" cy="4" r="1.1" />
      <circle cx="5.5" cy="8" r="1.1" />
      <circle cx="10.5" cy="8" r="1.1" />
      <circle cx="5.5" cy="12" r="1.1" />
      <circle cx="10.5" cy="12" r="1.1" />
    </svg>
  );
}

export function MoreDotsSvg(): ReactNode {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 16 16"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="3.5" cy="8" r="1.2" />
      <circle cx="8" cy="8" r="1.2" />
      <circle cx="12.5" cy="8" r="1.2" />
    </svg>
  );
}

export function PinIconSvg(): ReactNode {
  return (
    <svg
      width={18}
      height={18}
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M10 17S15 12.2 15 8.5A5 5 0 1 0 5 8.5C5 12.2 10 17 10 17Z"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <circle cx="10" cy="8.5" r="1.6" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}

export function DropArrowSvg(): ReactNode {
  return (
    <svg
      width={18}
      height={18}
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M10 4V15M10 15L5.5 10.5M10 15L14.5 10.5"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export function TeamIconSvg(): ReactNode {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="6" cy="5.5" r="2.25" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M2.2 13C2.6 10.8 4.1 9.6 6 9.6C7.9 9.6 9.4 10.8 9.8 13"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <circle cx="11.2" cy="6.2" r="1.75" stroke="currentColor" strokeWidth="1.3" />
      <path
        d="M10.2 9.9C11.9 10 13.3 11 13.8 13"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function ShieldCheckIconSvg(): ReactNode {
  return (
    <svg
      width={14}
      height={14}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M8 2L13 4V7.8C13 10.8 10.9 13.2 8 14C5.1 13.2 3 10.8 3 7.8V4L8 2Z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M5.6 8.1L7.3 9.7L10.6 6.4"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
