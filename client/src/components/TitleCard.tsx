// The preview's stand-ins for what the engine rasterises at export time: a
// full-frame title card and a caption box over the picture.

import type { CaptionEdit, TitleEdit } from '../types';

export function TitleCard({ title }: { title: TitleEdit }) {
  return (
    <div className={`title-card ${title.style}`} aria-live="polite">
      <div className="title-text">{title.text}</div>
      {title.subtitle && <div className="title-sub">{title.subtitle}</div>}
    </div>
  );
}

export function Caption({ caption }: { caption: CaptionEdit }) {
  return <div className={`caption ${caption.position}`}>{caption.text}</div>;
}
