// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { orderedPieces } from '../editlist';
import { timelineSegments } from '../timeline';
import type { Asset, Edit, Word } from '../types';
import { Timeline, type TimelineProps } from './Timeline';

const words: Word[] = Array.from({ length: 10 }, (_, i) => ({
  id: `w${i}`,
  text: `w${i}`,
  start: i,
  end: i + 0.5,
}));
const asset = (id: string, kind: 'video' | 'audio'): Asset => ({
  id,
  kind,
  name: `${id}.${kind === 'video' ? 'mp4' : 'mp3'}`,
  ext: kind === 'video' ? 'mp4' : 'mp3',
  duration: 30,
  width: null,
  height: null,
  createdAt: 0,
  url: `/data/m/assets/${id}`,
  poster: null,
});
const edits: Edit[] = [
  { kind: 'broll', start: 2, end: 4, media: 'a1', offset: 0 },
  { kind: 'audio', start: 0, end: 10, media: 'a2', offset: 0, gain: 0, duck: true },
];
const splits = [5];
const order = [5, 0];

function setup(overrides: Partial<TimelineProps> = {}): TimelineProps {
  const props: TimelineProps = {
    words,
    edits,
    assets: [asset('a1', 'video'), asset('a2', 'audio')],
    ordered: orderedPieces(10, edits, splits, order),
    segments: timelineSegments(10, edits, splits, order),
    outputTime: 0,
    peers: [],
    thumbs: null,
    readOnly: false,
    selectedClip: null,
    selectedOverlay: null,
    onSeek: vi.fn(),
    onSelectClip: vi.fn(),
    onMoveClip: vi.fn(),
    onSelectOverlay: vi.fn(),
    onOpenAudio: vi.fn(),
    tool: 'select',
    duration: 10,
    splits,
    onSplit: vi.fn(),
    onCut: vi.fn(),
    ...overrides,
  };
  render(<Timeline {...props} />);
  return props;
}

beforeEach(() => {
  // The lanes are 1,000 px wide over 10 s of output: 100 px per second.
  // Clip 0 is output [0, 5), clip 1 is output [5, 10).
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const clip = this.getAttribute('data-clip');
    if (clip !== null) return new DOMRect(Number(clip) * 500, 0, 500, 24);
    return new DOMRect(0, 0, 1000, 100);
  });
});

afterEach(() => vi.restoreAllMocks());

describe('Timeline (Select tool)', () => {
  it('draws one block per clip in output order, labelled with its first words', () => {
    setup();
    const clips = screen.getAllByRole('button', { name: /^Clip \d/ });
    expect(clips.map((c) => c.getAttribute('aria-label'))).toEqual([
      'Clip 1: w5 w6 w7 w8…',
      'Clip 2: w0 w1 w2 w3…',
    ]);
  });

  it('selects a clip on a plain click', () => {
    const props = setup();
    const first = screen.getByRole('button', { name: 'Clip 1: w5 w6 w7 w8…' });
    fireEvent.pointerDown(first, { clientX: 250, button: 0 });
    fireEvent.pointerUp(first, { clientX: 250, button: 0 });
    expect(props.onSelectClip).toHaveBeenCalledWith(5);
    expect(props.onMoveClip).not.toHaveBeenCalled();
  });

  it('moves a clip dragged in front of another', () => {
    const props = setup();
    const second = screen.getByRole('button', { name: 'Clip 2: w0 w1 w2 w3…' });
    fireEvent.pointerDown(second, { clientX: 750, button: 0 });
    fireEvent.pointerMove(second, { clientX: 100 });
    fireEvent.pointerUp(second, { clientX: 100 });
    expect(props.onMoveClip).toHaveBeenCalledWith(0, 5);
  });

  it('selects a focused clip from the keyboard', () => {
    const props = setup();
    // Enter or Space on a focused button fires a click with no pointer (detail 0).
    fireEvent.click(screen.getByRole('button', { name: 'Clip 2: w0 w1 w2 w3…' }), { detail: 0 });
    expect(props.onSelectClip).toHaveBeenCalledWith(0);
  });

  it('ignores the click that follows a pointer press on a clip', () => {
    const props = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Clip 2: w0 w1 w2 w3…' }), { detail: 1 });
    expect(props.onSelectClip).not.toHaveBeenCalled();
  });

  it('does not let a viewer drag clips', () => {
    const props = setup({ readOnly: true });
    const second = screen.getByRole('button', { name: 'Clip 2: w0 w1 w2 w3…' });
    fireEvent.pointerDown(second, { clientX: 750, button: 0 });
    fireEvent.pointerMove(second, { clientX: 100 });
    fireEvent.pointerUp(second, { clientX: 100 });
    expect(props.onMoveClip).not.toHaveBeenCalled();
  });

  it('selects a B-roll bar and opens a music bar on double-click', () => {
    const props = setup();
    fireEvent.click(screen.getByRole('button', { name: 'B-roll a1.mp4' }));
    expect(props.onSelectOverlay).toHaveBeenCalledWith({ kind: 'broll', start: 2 });
    fireEvent.doubleClick(screen.getByRole('button', { name: 'Music a2.mp3' }));
    expect(props.onOpenAudio).toHaveBeenCalledWith(0);
  });

  it('tags each overlay bar for the selection toolbar to anchor on', () => {
    setup();
    expect(screen.getByRole('button', { name: 'B-roll a1.mp4' }).getAttribute('data-overlay')).toBe(
      'broll:2',
    );
    expect(screen.getByRole('button', { name: 'Music a2.mp3' }).getAttribute('data-overlay')).toBe(
      'audio:0',
    );
  });

  it('seeks to the output time under the pointer', () => {
    const props = setup();
    // Output 2.5 s is inside the first clip (source 7.5); playback maps it.
    fireEvent.pointerDown(screen.getByTestId('timeline-lanes'), { clientX: 250, button: 0 });
    expect(props.onSeek).toHaveBeenCalledWith(2.5);
  });

  it('seeks to the output end at the right edge, not to the piece sharing its source instant', () => {
    const props = setup();
    fireEvent.pointerDown(screen.getByTestId('timeline-lanes'), { clientX: 1000, button: 0 });
    expect(props.onSeek).toHaveBeenCalledWith(10);
  });

  it('draws the playhead at the output time it is given', () => {
    setup({ outputTime: 7.5 });
    expect(screen.getByTestId('playhead').style.left).toBe('75%');
  });
});

describe('Timeline (Razor and Range)', () => {
  const lanes = () => screen.getByTestId('timeline-lanes');

  it('razor: shows where it will cut and splits at that word’s source time', () => {
    const props = setup({ tool: 'razor' });
    // Output 2.6 s snaps to output 3, the start of source word 8.
    fireEvent.pointerMove(lanes(), { clientX: 260 });
    expect(screen.getByTestId('razor-line').getAttribute('data-ok')).toBe('true');
    fireEvent.pointerDown(lanes(), { clientX: 260, button: 0 });
    expect(props.onSplit).toHaveBeenCalledWith(8);
    expect(props.onSeek).not.toHaveBeenCalled();
  });

  it('razor: refuses an existing piece start', () => {
    const props = setup({ tool: 'razor' });
    // Output 0.1 snaps to output 0: word 5, which is already the split at 5.
    fireEvent.pointerMove(lanes(), { clientX: 10 });
    expect(screen.getByTestId('razor-line').getAttribute('data-ok')).toBe('false');
    fireEvent.pointerDown(lanes(), { clientX: 10, button: 0 });
    expect(props.onSplit).not.toHaveBeenCalled();
  });

  it('razor: a press on a clip splits instead of dragging it', () => {
    const props = setup({ tool: 'razor' });
    const first = screen.getByRole('button', { name: 'Clip 1: w5 w6 w7 w8…' });
    fireEvent.pointerMove(lanes(), { clientX: 260 });
    fireEvent.pointerDown(first, { clientX: 260, button: 0 });
    expect(props.onSplit).toHaveBeenCalledWith(8);
    expect(props.onSelectClip).not.toHaveBeenCalled();
  });

  it('range: cuts the dragged stretch, snapped to word starts, in one call', () => {
    const props = setup({ tool: 'range' });
    fireEvent.pointerDown(lanes(), { clientX: 110, button: 0 });
    fireEvent.pointerMove(lanes(), { clientX: 390 });
    expect(screen.getByTestId('range-band')).toBeTruthy();
    fireEvent.pointerUp(lanes(), { clientX: 390 });
    // Output [1, 4) is source [6, 9).
    expect(props.onCut).toHaveBeenCalledTimes(1);
    expect(props.onCut).toHaveBeenCalledWith([{ start: 6, end: 9 }]);
  });

  it('razor: drops the line after a split, until the pointer moves again', () => {
    const props = setup({ tool: 'razor' });
    fireEvent.pointerMove(lanes(), { clientX: 260 });
    fireEvent.pointerDown(lanes(), { clientX: 260, button: 0 });
    expect(props.onSplit).toHaveBeenCalledWith(8);
    expect(screen.queryByTestId('razor-line')).toBeNull();
  });

  it('range: a cancelled drag abandons the band without cutting', () => {
    const props = setup({ tool: 'range' });
    fireEvent.pointerDown(lanes(), { clientX: 110, button: 0 });
    fireEvent.pointerMove(lanes(), { clientX: 390 });
    fireEvent.pointerCancel(lanes(), { clientX: 390 });
    expect(screen.queryByTestId('range-band')).toBeNull();
    expect(props.onCut).not.toHaveBeenCalled();
    expect(props.onSeek).not.toHaveBeenCalled();
  });

  it('opens a music bar on double-click only with the Select tool', () => {
    const props = setup({ tool: 'razor' });
    fireEvent.doubleClick(screen.getByRole('button', { name: 'Music a2.mp3' }));
    expect(props.onOpenAudio).not.toHaveBeenCalled();
  });

  it('shows no razor line with the Select tool', () => {
    setup();
    fireEvent.pointerMove(lanes(), { clientX: 260 });
    expect(screen.queryByTestId('razor-line')).toBeNull();
  });
});
