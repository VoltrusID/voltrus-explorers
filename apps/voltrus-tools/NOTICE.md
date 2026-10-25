# Third-party notices

Voltrus Tools is GPL-3.0-or-later (see the repository `LICENSE`). It includes or
adapts the following works under their own licences. The macOS app bundle
carries all of these texts in `Contents/Resources/Licenses`.

| Work | Used for | Licence |
| --- | --- | --- |
| [muxy](https://github.com/muxy-app/muxy), © 2026 Muxy | Crate layering, theme token derivation (`vt-ui/src/theme`), split layout model (`vt-app-core/src/layout.rs`), and the bundled Ghostty-format colour schemes (`vt-ui/assets/themes`, except `Voltrus` and `Voltrus Light`) | MIT |
| [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), Zed Industries, via the [muxy-app/gpui](https://github.com/muxy-app/gpui) fork kept at [VoltrusID/gpui](https://github.com/VoltrusID/gpui) | The UI framework; `vt-ui/src/text_input.rs` follows its `examples/input.rs` | Apache-2.0 (`packaging/licenses/gpui-LICENSE-APACHE.txt`) |
| [Lucide](https://lucide.dev) | Icons in `vt-ui/assets/icons` | ISC (`vt-ui/assets/icons/LICENSE-lucide.txt`) |
| [JetBrains Mono](https://www.jetbrains.com/lp/mono/) | Monospace face for values and addresses | SIL OFL 1.1 (`vt-ui/assets/fonts/OFL-JetBrainsMono.txt`) |
| [iTerm2 Color Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes), © 2011 to present Mark Badolato | Origin of most bundled colour schemes, as distributed with Ghostty | MIT (below) |

The MIT notice required by muxy:

```
MIT License

Copyright (c) 2026 Muxy

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

The MIT notice required by iTerm2 Color Schemes (individual themes remain
their authors'):

```
MIT License

Copyright (c) 2011 to Present Mark Badolato

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

This license covers the iTerm-Color-Schemes repository collection of themes. 

The copyright/license for each individual theme belongs to the author of that theme.
```
