// Driver: native engine when the addon loads, the pure-JS drop-in (solve.js) otherwise.
// Outside a container MAKEFUN_INPUT_DIR and MAKEFUN_OUTPUT_DIR stand in for /input and /output.

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { setFlagsFromString } from 'node:v8';

setFlagsFromString('--no-incremental-marking');

const INPUT_DIR = process.env.MAKEFUN_INPUT_DIR ?? '/input';
const OUTPUT_DIR = process.env.MAKEFUN_OUTPUT_DIR ?? '/output';

let native: { solveAll?: (text: string) => string } | null = null;
try {
  native = createRequire(import.meta.url)('./engine.node') as { solveAll?: (text: string) => string };
} catch {
  native = null;
}

if (native !== null && typeof native.solveAll === 'function') {
  const text = readFileSync(join(INPUT_DIR, 'cases.json'), 'utf8');
  writeFileSync(join(OUTPUT_DIR, 'answers.json'), native.solveAll(text));
} else {
  const { run } = (await import(new URL('./solve.js', import.meta.url).href)) as { run: (input: unknown) => unknown };
  const cases = JSON.parse(readFileSync(join(INPUT_DIR, 'cases.json'), 'utf8')) as { id: string; input: unknown }[];
  const answers: unknown[] = [];
  for (const c of cases) answers.push(run(c.input));
  writeFileSync(join(OUTPUT_DIR, 'answers.json'), JSON.stringify(answers));
}
