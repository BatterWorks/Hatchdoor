import { useEffect, useMemo, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import { NavLink, Route, Routes, useLocation, useNavigate, useParams } from 'react-router-dom'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import mermaid from 'mermaid'
import './App.css'

type ExplorerFolder = {
  name: string
  folders: ExplorerFolder[]
  notes: ExplorerNote[]
}

type ExplorerNote = {
  title: string
  slug: string
}

type Note = {
  title: string
  slug: string
  content: string
}

function App() {
  const [tree, setTree] = useState<ExplorerFolder | null>(null)
  const [loadingTree, setLoadingTree] = useState(true)
  const [treeError, setTreeError] = useState<string | null>(null)
  const location = useLocation()
  const navigate = useNavigate()

  useEffect(() => {
    void (async () => {
      setLoadingTree(true)
      setTreeError(null)
      try {
        const res = await fetch('/api/tree')
        if (!res.ok) {
          throw new Error(`Failed loading tree: ${res.status}`)
        }
        setTree((await res.json()) as ExplorerFolder)
      } catch (err) {
        setTreeError(err instanceof Error ? err.message : 'Unknown tree loading error')
      } finally {
        setLoadingTree(false)
      }
    })()
  }, [])

  return (
    <div className="app-layout">
      <aside className="explorer-pane">
        <header className="explorer-header">
          <h1>Hatchdoor</h1>
          <p>Vault Explorer</p>
          <button className="close-note" onClick={() => navigate('/')}>Close Note</button>
        </header>
        {loadingTree ? <p>Loading explorer…</p> : null}
        {treeError ? <p className="error">{treeError}</p> : null}
        {tree ? <FolderTree root={tree} currentPath={location.pathname} /> : null}
      </aside>

      <main className="note-pane">
        <Routes>
          <Route path="/" element={<EmptyState />} />
          <Route path="/n/:slug" element={<NotePage />} />
        </Routes>
      </main>
    </div>
  )
}

function EmptyState() {
  return (
    <section>
      <h2>Notes Explorer</h2>
      <p>Select any note from the left panel to open it.</p>
    </section>
  )
}

function FolderTree({ root, currentPath }: { root: ExplorerFolder; currentPath: string }) {
  return (
    <ul className="tree root-tree">
      {root.folders.map((folder) => (
        <FolderNode key={`folder-${folder.name}`} folder={folder} currentPath={currentPath} />
      ))}
      {root.notes.map((note) => (
        <NoteNode key={note.slug} note={note} currentPath={currentPath} />
      ))}
    </ul>
  )
}

function FolderNode({ folder, currentPath }: { folder: ExplorerFolder; currentPath: string }) {
  return (
    <li className="folder-item">
      <details open>
        <summary>{folder.name}</summary>
        <ul className="tree">
          {folder.folders.map((child) => (
            <FolderNode key={`${folder.name}-${child.name}`} folder={child} currentPath={currentPath} />
          ))}
          {folder.notes.map((note) => (
            <NoteNode key={note.slug} note={note} currentPath={currentPath} />
          ))}
        </ul>
      </details>
    </li>
  )
}

function NoteNode({ note, currentPath }: { note: ExplorerNote; currentPath: string }) {
  return (
    <li className="note-item">
      <NavLink className={currentPath === `/n/${note.slug}` ? 'note-link active-note' : 'note-link'} to={`/n/${note.slug}`}>
        {note.title}
      </NavLink>
    </li>
  )
}

function NotePage() {
  const params = useParams<{ slug: string }>()
  const slug = params.slug ?? ''
  const [note, setNote] = useState<Note | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    void (async () => {
      setLoading(true)
      setError(null)
      setNote(null)
      try {
        const res = await fetch(`/api/note/${encodeURIComponent(slug)}`)
        if (!res.ok) {
          throw new Error(`Failed loading note: ${res.status}`)
        }
        const json = (await res.json()) as { note: Note }
        setNote(json.note)
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Unknown note loading error')
      } finally {
        setLoading(false)
      }
    })()
  }, [slug])

  const markdown = useResolvedWikilinks(note?.content ?? '')

  if (loading) {
    return <p>Loading note…</p>
  }
  if (error) {
    return <p className="error">{error}</p>
  }
  if (!note) {
    return <p className="error">Note not found.</p>
  }

  return (
    <article>
      <h2>{note.title}</h2>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          code(props) {
            const { children, className } = props
            const match = /language-(\w+)/.exec(className || '')
            if (match?.[1] === 'mermaid') {
              return <MermaidDiagram chart={String(children).trim()} />
            }
            return <code className={className}>{children}</code>
          },
        }}
      >
        {markdown}
      </ReactMarkdown>
    </article>
  )
}

function useResolvedWikilinks(markdown: string): string {
  const [resolved, setResolved] = useState(markdown)

  useEffect(() => {
    if (!markdown) {
      queueMicrotask(() => setResolved(''))
      return
    }

    let cancelled = false

    void (async () => {
      const matches = [...markdown.matchAll(/\[\[([^\]]+)\]\]/g)]
      const rawTargets = matches
        .map((m) => parseWikilinkTarget(m[1]).target)
        .filter((target) => target.length > 0)
      const uniqueTargets = [...new Set(rawTargets)]

      const map = new Map<string, string | null>()
      await Promise.all(
        uniqueTargets.map(async (target) => {
          try {
            const res = await fetch(`/api/resolve?target=${encodeURIComponent(target)}`)
            if (!res.ok) {
              map.set(target, null)
              return
            }
            const json = (await res.json()) as { slug: string | null }
            map.set(target, json.slug)
          } catch {
            map.set(target, null)
          }
        }),
      )

      const rewritten = markdown.replace(/\[\[([^\]]+)\]\]/g, (_whole, body: string) => {
        const parsed = parseWikilinkTarget(body)
        const slug = map.get(parsed.target)
        if (slug) {
          return `[${parsed.label}](/n/${slug})`
        }
        return `**${parsed.label}**`
      })

      if (!cancelled) {
        setResolved(rewritten)
      }
    })()

    return () => {
      cancelled = true
    }
  }, [markdown])

  return useMemo(() => resolved, [resolved])
}

function parseWikilinkTarget(body: string): { target: string; label: string } {
  const [targetRaw, aliasRaw] = body.split('|', 2)
  const target = (targetRaw || '').trim()
  const label = (aliasRaw || '').trim() || target
  return { target, label }
}

function MermaidDiagram({ chart }: { chart: string }) {
  const [svg, setSvg] = useState<string>('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' })
  }, [])

  useEffect(() => {
    let mounted = true
    const id = `m-${Math.random().toString(36).slice(2)}`

    void (async () => {
      try {
        const { svg: rendered } = await mermaid.render(id, chart)
        if (mounted) {
          setSvg(rendered)
          setError(null)
        }
      } catch (err) {
        if (mounted) {
          setError(err instanceof Error ? err.message : 'Invalid mermaid diagram')
        }
      }
    })()

    return () => {
      mounted = false
    }
  }, [chart])

  if (error) {
    return <pre className="error">Mermaid error: {error}</pre>
  }

  return <div className="mermaid" dangerouslySetInnerHTML={{ __html: svg }} />
}

export default App
