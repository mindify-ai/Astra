import { memo, type ReactNode, useState } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import rehypeHighlight from 'rehype-highlight'
import remarkGfm from 'remark-gfm'
import { cx } from './cx'

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (node && typeof node === 'object' && 'props' in node)
    return textOf((node as { props: { children?: ReactNode } }).props.children)
  return ''
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false)
  const copy = () => {
    void navigator.clipboard
      ?.writeText(textOf(children).replace(/\n$/, ''))
      .then(() => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1200)
      })
      .catch(() => {})
  }
  return (
    <div className="group relative my-2">
      <pre className="m-0 overflow-x-auto rounded-sm border border-line bg-surface-sunken p-3 font-mono text-code">
        {children}
      </pre>
      <button
        type="button"
        onClick={copy}
        className="absolute top-1.5 right-1.5 rounded-sm border border-line bg-surface-raised px-2 py-0.5 font-mono text-spec uppercase text-ink-muted opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}

const components: Components = {
  p: ({ node: _, ...p }) => <p className="my-2 first:mt-0 last:mb-0" {...p} />,
  h1: ({ node: _, ...p }) => <h3 className="mt-4 mb-2 text-heading first:mt-0" {...p} />,
  h2: ({ node: _, ...p }) => <h4 className="mt-4 mb-2 text-heading first:mt-0" {...p} />,
  h3: ({ node: _, ...p }) => <h5 className="mt-3 mb-1 text-body font-semibold first:mt-0" {...p} />,
  h4: ({ node: _, ...p }) => <h6 className="mt-3 mb-1 text-body font-semibold first:mt-0" {...p} />,
  h5: ({ node: _, ...p }) => <h6 className="mt-3 mb-1 text-body font-semibold first:mt-0" {...p} />,
  h6: ({ node: _, ...p }) => <h6 className="mt-3 mb-1 text-body font-semibold first:mt-0" {...p} />,
  ul: ({ node: _, className, ...p }) => (
    <ul
      className={cx(
        'my-2 pl-5',
        className?.includes('contains-task-list') ? 'list-none pl-1' : 'list-disc',
        className,
      )}
      {...p}
    />
  ),
  ol: ({ node: _, ...p }) => <ol className="my-2 list-decimal pl-5" {...p} />,
  li: ({ node: _, ...p }) => (
    <li className="my-0.5 [&>input]:mr-2 [&>input]:align-middle [&>input]:accent-signal" {...p} />
  ),
  a: ({ node: _, ...p }) => (
    <a
      className="text-signal underline underline-offset-2"
      target="_blank"
      rel="noreferrer noopener"
      {...p}
    />
  ),
  blockquote: ({ node: _, ...p }) => (
    <blockquote className="my-2 border-l-2 border-line-strong pl-3 text-ink-muted" {...p} />
  ),
  hr: () => <hr className="my-3 border-0 border-t border-line" />,
  pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
  code: ({ node: _, className, ...p }) =>
    // Fenced blocks carry a language class (or sit inside <pre>, styled there); inline code gets a chip.
    className ? (
      <code className={className} {...p} />
    ) : (
      <code
        className="rounded-sm border border-line-subtle bg-surface-sunken px-1 py-px font-mono text-[0.92em] [pre_&]:border-0 [pre_&]:bg-transparent [pre_&]:p-0"
        {...p}
      />
    ),
  table: ({ node: _, ...p }) => (
    <div className="my-2 overflow-x-auto">
      <table className="border-collapse text-body-sm" {...p} />
    </div>
  ),
  th: ({ node: _, ...p }) => (
    <th className="border border-line bg-surface-raised px-2 py-1 text-left font-semibold" {...p} />
  ),
  td: ({ node: _, ...p }) => <td className="border border-line px-2 py-1 align-top" {...p} />,
  img: ({ node: _, alt }) => <span className="text-ink-muted">[image{alt ? `: ${alt}` : ''}]</span>,
}

/**
 * Markdown as agents and people write it: GitHub-flavoured (tables, task lists, strikethrough) with
 * highlighted code blocks. Raw HTML is not rendered, and images show as their alt text (agent output
 * must not load remote content).
 */
export const Markdown = memo(function Markdown({
  children,
  className,
}: {
  children: string
  className?: string
}) {
  return (
    <div className={cx('astra-md min-w-0 break-words', className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: false }]]}
        components={components}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
})
