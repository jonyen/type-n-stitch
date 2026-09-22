// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';

import { shouldIgnoreGlobalKey } from './keyboardGuard';

describe('shouldIgnoreGlobalKey', () => {
  it('ignores when the event was already handled', () => {
    expect(shouldIgnoreGlobalKey(null, true)).toBe(true);
  });

  it('does not ignore a bare target with nothing preventing it', () => {
    const div = document.createElement('div');
    document.body.append(div);
    expect(shouldIgnoreGlobalKey(div, false)).toBe(false);
  });

  it('ignores a null target that has not been prevented', () => {
    expect(shouldIgnoreGlobalKey(null, false)).toBe(false);
  });

  it('ignores form controls and contenteditable', () => {
    for (const html of [
      '<input>',
      '<textarea></textarea>',
      '<select></select>',
      '<div contenteditable="true"></div>',
    ]) {
      const wrap = document.createElement('div');
      wrap.innerHTML = html;
      const el = wrap.firstElementChild;
      if (!el) throw new Error('fixture missing');
      document.body.append(wrap);
      expect(shouldIgnoreGlobalKey(el, false)).toBe(true);
    }
  });

  it('ignores anything inside a Radix menu, menubar, listbox or popper wrapper', () => {
    for (const role of ['menu', 'menubar', 'listbox']) {
      const menu = document.createElement('div');
      menu.setAttribute('role', role);
      const item = document.createElement('div');
      menu.append(item);
      document.body.append(menu);
      expect(shouldIgnoreGlobalKey(item, false)).toBe(true);
    }
    const wrapper = document.createElement('div');
    wrapper.setAttribute('data-radix-popper-content-wrapper', '');
    const inner = document.createElement('span');
    wrapper.append(inner);
    document.body.append(wrapper);
    expect(shouldIgnoreGlobalKey(inner, false)).toBe(true);
  });
});
