# Third-Party Notices

## Windows Console ConPTY (conpty.dll / OpenConsole.exe)

Windows 版の oretachi は、全角文字の欠落を避けるため、次の Microsoft 製バイナリを exe の隣に同梱する (#421, #423)。

- パッケージ: `Microsoft.Windows.Console.ConPTY` 1.25.260930003
- 入手元: https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY (ソース: https://github.com/microsoft/terminal)
- 取得: `scripts/fetch-conpty.mjs` がビルド時に SHA256 を検証して取得する (リポジトリには含めない)
- ライセンス: MIT

```
Copyright (c) Microsoft Corporation. All rights reserved.

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED *AS IS*, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

```
