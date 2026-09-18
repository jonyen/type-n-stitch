import { clipLength, relativeTime } from '../format';
import type { ProjectSummary } from '../types';

interface Props {
  items: ProjectSummary[];
  onOpen: (project: ProjectSummary) => void;
  disabled: boolean;
}

/** The signed-in user's projects, newest first; hidden until there is one. */
export function Projects({ items, onOpen, disabled }: Props) {
  if (items.length === 0) return null;
  return (
    <section className="library projects" aria-labelledby="projects-heading">
      <div className="library-head">
        <h2 id="projects-heading">Your projects</h2>
        <span className="muted">
          {items.length} {items.length === 1 ? 'project' : 'projects'}
        </span>
      </div>
      <ul className="library-grid">
        {items.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              className="library-card project-card"
              disabled={disabled}
              onClick={() => onOpen(p)}
            >
              <span className={`poster ${p.media.kind}`}>
                <span className="poster-glyph" aria-hidden>
                  {p.media.kind === 'audio' ? '♪' : '▶'}
                </span>
                <span className="length">{clipLength(p.media.duration)}</span>
              </span>
              <strong>{p.title}</strong>
              <span className="project-meta">
                <span className={`role ${p.role}`}>{p.role}</span>
                <span className="muted">opened {relativeTime(p.createdAt)}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
