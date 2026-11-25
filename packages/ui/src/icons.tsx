import type { SVGProps } from 'react'

type IconProps = SVGProps<SVGSVGElement> & { size?: number }

const stroke = (size: number, props: IconProps) => ({
  'aria-hidden': true as const,
  width: size,
  height: size,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  ...props,
})

/** The Astra mark: a planet and its moon. */
export function LogoMark({ size = 20, ...props }: IconProps) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" {...props}>
      <circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="19" cy="8" r="2.5" fill="currentColor" />
    </svg>
  )
}

export const CheckIcon = ({ size = 14, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M20 6 9 17l-5-5" />
  </svg>
)
export const ArrowRightIcon = ({ size = 14, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M5 12h14M13 6l6 6-6 6" />
  </svg>
)
export const CloseIcon = ({ size = 16, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M6 6l12 12M18 6 6 18" />
  </svg>
)
export const ChevronLeftIcon = ({ size = 24, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M15 18l-6-6 6-6" />
  </svg>
)
export const ArrowLeftIcon = ({ size = 24, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M12 19l-7-7 7-7M19 12H5" />
  </svg>
)
export const ArrowUpIcon = ({ size = 20, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M12 19V5M5 12l7-7 7 7" />
  </svg>
)
export const HomeIcon = ({ size = 20, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M3 10.5 12 3l9 7.5V21h-6v-6H9v6H3z" />
  </svg>
)
export const ListIcon = ({ size = 20, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" />
  </svg>
)
export const ChipIcon = ({ size = 20, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <rect x="4" y="4" width="16" height="16" />
    <rect x="9" y="9" width="6" height="6" />
  </svg>
)
export const UsersIcon = ({ size = 20, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
    <circle cx="9" cy="7" r="4" />
    <path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75" />
  </svg>
)
export const ReloadIcon = ({ size = 16, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4" />
  </svg>
)
export const ExternalIcon = ({ size = 16, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />
  </svg>
)
export const GlobeIcon = ({ size = 16, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9z" />
  </svg>
)
export const ChatIcon = ({ size = 16, ...p }: IconProps) => (
  <svg {...stroke(size, p)}>
    <path d="M4 5h16v11H9l-5 4z" />
  </svg>
)
