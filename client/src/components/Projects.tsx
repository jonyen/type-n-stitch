import { formatTime } from '../editlist';
import type { ProjectSummary } from '../types';

interface Props {
  items: ProjectSummary[];
  onOpen: (project: ProjectSummary) => void;
}

export function Projects({ items, onOpen }: Props) {
  if (items.length === 0) return null;
  return (
    <section className="projects">
      <h2>Your projects</h2>
      <ul>
        {items.map((p) => (
          <li key={p.id}>
            <button type="button" onClick={() => onOpen(p)}>
              <span className="title">{p.title}</span>
              <span className="muted">
                {p.media.kind} · {formatTime(p.media.duration)} · {p.role}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
