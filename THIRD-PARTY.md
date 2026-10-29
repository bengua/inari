# Third-party code

## OpenFox

Parts of Inari's local engine (`crates/inari-core/src/engine/`: the system prompt, the builder and
planner agents, the compaction prompt, and the tools' names, parameters and descriptions) are taken
from OpenFox 2.0.161, https://github.com/co-l/openfox, and adapted to Inari.

OpenFox is published under the MIT licence (its `package.json` declares `"license": "MIT"`; the
repository carries no separate licence file and names no copyright holder, so the notice below
names its authors as a whole):

```
MIT License

Copyright (c) the OpenFox authors (https://github.com/co-l/openfox)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
