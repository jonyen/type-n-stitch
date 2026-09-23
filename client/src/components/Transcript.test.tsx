// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Word } from '../types';
import { Transcript } from './Transcript';

const words: Word[] = ['one', 'two', 'three'].map((text, i) => ({
  id: `w${i}`,
  text,
  start: i,
  end: i + 0.5,
}));

function setup() {
  const onWordDrag = vi.fn();
  const onWordDragEnd = vi.fn();
  render(
    <Transcript
      words={words}
      edits={[]}
      selected={null}
      activeWord={-1}
      playing={false}
      showCuts
      onWordClick={vi.fn()}
      onWordDrag={onWordDrag}
      onOverdubClick={vi.fn()}
      selectedTitle={null}
      onTitleClick={vi.fn()}
      onTitleOpen={vi.fn()}
      onCaptionClick={vi.fn()}
      onCutTransition={vi.fn()}
      speakers={null}
      speakerNames={[]}
      onRenameSpeaker={vi.fn()}
      readOnly={false}
      peers={[]}
      ordered={[{ start: 0, end: 3 }]}
      splits={[]}
      selectedClip={null}
      onClipClick={vi.fn()}
      assets={[]}
      onBrollClick={vi.fn()}
      onAudioClick={vi.fn()}
      tool="range"
      onWordDragEnd={onWordDragEnd}
    />,
  );
  return { onWordDrag, onWordDragEnd };
}

describe('Transcript word drag (the Range tool cuts on its end)', () => {
  it('a plain click is no drag: nothing ends, so nothing is cut', () => {
    const { onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'two' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).not.toHaveBeenCalled();
  });

  it('a press dragged across words ends the drag on release', () => {
    const { onWordDrag, onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'one' }), { button: 0 });
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'two' }));
    fireEvent.mouseUp(window);
    expect(onWordDrag).toHaveBeenCalledWith(1);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
    // The next plain click starts over.
    fireEvent.mouseDown(screen.getByRole('button', { name: 'three' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
  });
});
