// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

/**
 * Parse the `// @name: <value>` front-matter from a Rhai script source.
 * Returns the name string, or '' if not found.
 */
export function parseFrontMatterName(source: string): string {
  for (const line of source.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed.startsWith('//')) {
      if (trimmed === '') continue;
      break;
    }
    const body = trimmed.replace(/^\/\/\s*/, '');
    if (body.startsWith('@name:')) {
      return body.slice('@name:'.length).trim();
    }
  }
  return '';
}
