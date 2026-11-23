import type { ComponentPropsWithoutRef, ReactNode } from 'react'
import { useId } from 'react'
import { cx } from './cx'

// Inputs: raised ground, line-strong edge, 2px radius. 40px on desktop, 48px on phones.

export const controlClass = (
  opts: { size?: 'sm' | 'md' | 'touch'; mono?: boolean; ground?: 'raised' | 'page' } = {},
) =>
  cx(
    'box-border min-w-0 rounded-sm border border-line-strong text-ink',
    opts.ground === 'page' ? 'bg-surface' : 'bg-surface-raised',
    opts.size === 'sm' ? 'h-8 px-2' : opts.size === 'touch' ? 'h-12 px-3 text-[16px]' : 'h-10 px-3',
    opts.mono ? 'font-mono text-data' : opts.size === 'touch' ? '' : 'text-body-sm',
  )

export function Field({
  label,
  children,
  className,
}: {
  label: ReactNode
  children: (id: string) => ReactNode
  className?: string
}) {
  const id = useId()
  return (
    <div className={cx('flex flex-col gap-1.5', className)}>
      <label htmlFor={id} className="text-[13px] leading-5 text-ink-muted">
        {label}
      </label>
      {children(id)}
    </div>
  )
}

export function TextInput({
  size,
  mono,
  className,
  ...props
}: Omit<ComponentPropsWithoutRef<'input'>, 'size'> & { size?: 'sm' | 'md' | 'touch'; mono?: boolean }) {
  return <input className={cx(controlClass({ size, mono }), className)} {...props} />
}

export function Select({
  size,
  mono,
  ground,
  className,
  ...props
}: Omit<ComponentPropsWithoutRef<'select'>, 'size'> & {
  size?: 'sm' | 'md' | 'touch'
  mono?: boolean
  ground?: 'raised' | 'page'
}) {
  return <select className={cx(controlClass({ size, mono, ground }), className)} {...props} />
}

export function TextArea({ className, ...props }: ComponentPropsWithoutRef<'textarea'>) {
  return (
    <textarea
      className={cx(
        'box-border min-h-22 resize-y rounded-sm border border-line-strong bg-surface-raised px-3 py-2.5 text-body text-ink',
        className,
      )}
      {...props}
    />
  )
}

/** A radio presented as a selectable card (Launch: "Split the work" / "Race"). */
export function RadioCard({
  name,
  checked,
  onChange,
  title,
  description,
}: {
  name: string
  checked: boolean
  onChange: () => void
  title: ReactNode
  description: ReactNode
}) {
  return (
    <label
      className={cx(
        'flex flex-[1_1_280px] cursor-pointer items-start gap-3 border bg-surface-raised p-4',
        checked ? 'border-ink' : 'border-line',
      )}
    >
      <input type="radio" name={name} checked={checked} onChange={onChange} className="mt-1" />
      <span className="flex flex-col">
        <b className="font-semibold">{title}</b>
        <span className="text-ink-muted">{description}</span>
      </span>
    </label>
  )
}

export function Radio({
  name,
  checked,
  onChange,
  children,
}: {
  name: string
  checked: boolean
  onChange: () => void
  children: ReactNode
}) {
  return (
    <label className="flex min-h-8 cursor-pointer items-center gap-2">
      <input type="radio" name={name} checked={checked} onChange={onChange} />
      {children}
    </label>
  )
}

export function Fieldset({
  legend,
  children,
  className,
}: {
  legend: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <fieldset className={cx('m-0 flex flex-col border-0 p-0', className)}>
      <legend className="mb-1.5 p-0 text-[13px] leading-5 text-ink-muted">{legend}</legend>
      {children}
    </fieldset>
  )
}
