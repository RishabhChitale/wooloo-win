"""A small VT100/xterm screen emulator, no dependencies.

Feeds on the raw byte stream of `hc attach --observe --json` and keeps a grid of cells whose colors and
modifiers are already in Herdr's surface encoding, so a client shim can send them to wooloo as they are:
  color: 0 default, (0<<24)|n for ANSI 1..16 (n = index + 1), (1<<24)|n for the 256 palette, (2<<24)|rgb
  modifier: Ratatui bits - bold 1, dim 2, italic 4, underlined 8, reversed 64, hidden 128, crossed out 256
Only what terminal programs such as Claude Code use is handled; anything else is ignored, never fatal."""
import codecs
import unicodedata

BOLD, DIM, ITALIC, UNDER, REVERSE, HIDDEN, STRIKE = 1, 2, 4, 8, 64, 128, 256


def char_width(ch):
    if ch < " ":
        return 0
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = max(1, cols), max(1, rows)
        self.dec = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self.state = "g"
        self.buf = ""
        self.reset()

    # ---- state -------------------------------------------------------------------------------
    def blank(self):
        return (" ", 0, 0, 0, 0)  # symbol, fg, bg, modifier, skip

    def new_grid(self):
        return [[self.blank() for _ in range(self.cols)] for _ in range(self.rows)]

    def reset(self):
        self.grid = self.new_grid()
        self.alt = None
        self.x = self.y = 0
        self.pending = False
        self.fg = self.bg = self.mod = 0
        self.top, self.bottom = 0, self.rows - 1
        self.saved = (0, 0, 0, 0, 0)
        self.cursor_visible = True
        self.autowrap = True
        self.tabs = set(range(8, self.cols, 8))

    def resize(self, cols, rows):
        cols, rows = max(1, cols), max(1, rows)
        if (cols, rows) == (self.cols, self.rows):
            return
        grid = []
        for y in range(rows):
            old = self.grid[y] if y < len(self.grid) else None
            row = [old[x] if old and x < len(old) else self.blank() for x in range(cols)]
            grid.append(row)
        self.cols, self.rows, self.grid = cols, rows, grid
        self.alt = None
        self.x, self.y = min(self.x, cols - 1), min(self.y, rows - 1)
        self.top, self.bottom = 0, rows - 1
        self.pending = False
        self.tabs = set(range(8, cols, 8))

    # ---- feeding -----------------------------------------------------------------------------
    def feed(self, data):
        text = self.dec.decode(data)
        i, n = 0, len(text)
        while i < n:
            ch = text[i]
            st = self.state
            if st == "g":
                if ch == "\x1b":
                    self.state = "e"
                elif ch < " " or ch == "\x7f":
                    self.control(ch)
                else:
                    j = i
                    while j < n and text[j] >= " " and text[j] != "\x7f" and text[j] != "\x1b":
                        j += 1
                    self.put_text(text[i:j])
                    i = j
                    continue
            elif st == "e":
                self.escape(ch)
            elif st == "c":  # CSI
                if "0" <= ch <= "?" :
                    self.buf += ch
                elif " " <= ch <= "/":
                    self.buf += ch
                elif "@" <= ch <= "~":
                    try:
                        self.csi(self.buf, ch)
                    except Exception:
                        pass
                    self.buf = ""
                    self.state = "g"
                elif ch == "\x1b":
                    self.buf, self.state = "", "e"
                elif ch < " ":
                    self.control(ch)
                else:
                    self.buf, self.state = "", "g"
            elif st == "o":  # OSC and other string sequences: skip to BEL or ST
                if ch == "\x07":
                    self.state = "g"
                elif ch == "\x1b":
                    self.state = "os"
            elif st == "os":
                self.state = "g" if ch == "\\" else ("o" if ch != "\x1b" else "os")
            elif st == "x":  # one more character to swallow (charset, DEC test)
                self.state = "g"
            i += 1

    def control(self, ch):
        if ch == "\n" or ch == "\x0b" or ch == "\x0c":
            self.index()
        elif ch == "\r":
            self.x, self.pending = 0, False
        elif ch == "\b":
            self.x, self.pending = max(0, self.x - 1), False
        elif ch == "\t":
            nx = min((t for t in self.tabs if t > self.x), default=self.cols - 1)
            self.x, self.pending = nx, False

    def escape(self, ch):
        self.state = "g"
        if ch == "[":
            self.state, self.buf = "c", ""
        elif ch in "]PX^_":
            self.state = "o"
        elif ch in "()*+#%":
            self.state = "x"
        elif ch == "7":
            self.saved = (self.x, self.y, self.fg, self.bg, self.mod)
        elif ch == "8":
            self.x, self.y, self.fg, self.bg, self.mod = self.saved
            self.x, self.y = min(self.x, self.cols - 1), min(self.y, self.rows - 1)
            self.pending = False
        elif ch == "D":
            self.index()
        elif ch == "E":
            self.x, self.pending = 0, False
            self.index()
        elif ch == "M":
            self.reverse_index()
        elif ch == "H":
            self.tabs.add(self.x)
        elif ch == "c":
            self.reset()

    # ---- drawing -----------------------------------------------------------------------------
    def put_text(self, text):
        for ch in text:
            w = char_width(ch)
            if w == 0:
                # a combining mark joins the previous cell
                px = self.x - 1 if not self.pending else self.x
                if 0 <= px < self.cols and self.y < self.rows:
                    s, fg, bg, mod, sk = self.grid[self.y][px]
                    self.grid[self.y][px] = (s + ch, fg, bg, mod, sk)
                continue
            if self.pending or self.x + w > self.cols:
                if self.autowrap:
                    self.x, self.pending = 0, False
                    self.index()
                else:
                    self.x = self.cols - w
            row = self.grid[self.y]
            row[self.x] = (ch, self.fg, self.bg, self.mod, 0)
            if w == 2 and self.x + 1 < self.cols:
                row[self.x + 1] = ("", self.fg, self.bg, self.mod, 1)
            self.x += w
            if self.x >= self.cols:
                self.x, self.pending = self.cols - 1, True

    def index(self):
        if self.y == self.bottom:
            self.scroll_up(1)
        elif self.y < self.rows - 1:
            self.y += 1
        self.pending = False

    def reverse_index(self):
        if self.y == self.top:
            self.scroll_down(1)
        elif self.y > 0:
            self.y -= 1
        self.pending = False

    def erase_cell(self):
        return (" ", 0, self.bg, 0, 0)

    def scroll_up(self, n):
        n = max(1, min(n, self.bottom - self.top + 1))
        for _ in range(n):
            del self.grid[self.top]
            self.grid.insert(self.bottom, [self.erase_cell() for _ in range(self.cols)])

    def scroll_down(self, n):
        n = max(1, min(n, self.bottom - self.top + 1))
        for _ in range(n):
            del self.grid[self.bottom]
            self.grid.insert(self.top, [self.erase_cell() for _ in range(self.cols)])

    # ---- CSI ---------------------------------------------------------------------------------
    def csi(self, raw, final):
        private = raw[:1] in ("?", ">", "=", "<")
        body = raw[1:] if private else raw
        inter = ""
        while body and body[-1] in " !\"#$%&'()*+,-./":
            inter = body[-1] + inter
            body = body[:-1]
        nums = []
        for part in body.split(";"):
            if part == "":
                nums.append(None)
            else:
                try:
                    nums.append(int(part.split(":")[0]))
                except ValueError:
                    nums.append(None)

        def arg(i, default):
            v = nums[i] if i < len(nums) else None
            return default if v in (None, 0) else v

        if final == "m" and not private:
            self.sgr(body)
            return
        if private:
            if final in ("h", "l"):
                on = final == "h"
                for v in nums:
                    if v == 25:
                        self.cursor_visible = on
                    elif v == 7:
                        self.autowrap = on
                    elif v in (1049, 1047, 47):
                        self.alt_screen(on, v == 1049)
            return
        if inter == "!" and final == "p":
            self.fg = self.bg = self.mod = 0
            self.top, self.bottom = 0, self.rows - 1
            self.autowrap, self.cursor_visible = True, True
            return
        if final == "A":
            self.y = max(self.top if self.y >= self.top else 0, self.y - arg(0, 1)); self.pending = False
        elif final == "B" or final == "e":
            self.y = min(self.bottom if self.y <= self.bottom else self.rows - 1, self.y + arg(0, 1)); self.pending = False
        elif final == "C" or final == "a":
            self.x = min(self.cols - 1, self.x + arg(0, 1)); self.pending = False
        elif final == "D":
            self.x = max(0, self.x - arg(0, 1)); self.pending = False
        elif final == "E":
            self.y = min(self.rows - 1, self.y + arg(0, 1)); self.x = 0; self.pending = False
        elif final == "F":
            self.y = max(0, self.y - arg(0, 1)); self.x = 0; self.pending = False
        elif final == "G" or final == "`":
            self.x = min(self.cols - 1, arg(0, 1) - 1); self.pending = False
        elif final == "H" or final == "f":
            self.y = min(self.rows - 1, arg(0, 1) - 1); self.x = min(self.cols - 1, arg(1, 1) - 1); self.pending = False
        elif final == "d":
            self.y = min(self.rows - 1, arg(0, 1) - 1); self.pending = False
        elif final == "J":
            mode = nums[0] if nums and nums[0] is not None else 0
            if mode == 0:
                self.clear_range(self.y, self.x, self.cols)
                for y in range(self.y + 1, self.rows): self.clear_range(y, 0, self.cols)
            elif mode == 1:
                for y in range(0, self.y): self.clear_range(y, 0, self.cols)
                self.clear_range(self.y, 0, self.x + 1)
            else:
                for y in range(self.rows): self.clear_range(y, 0, self.cols)
        elif final == "K":
            mode = nums[0] if nums and nums[0] is not None else 0
            if mode == 0: self.clear_range(self.y, self.x, self.cols)
            elif mode == 1: self.clear_range(self.y, 0, self.x + 1)
            else: self.clear_range(self.y, 0, self.cols)
        elif final == "L":
            if self.top <= self.y <= self.bottom:
                n = min(arg(0, 1), self.bottom - self.y + 1)
                for _ in range(n):
                    del self.grid[self.bottom]
                    self.grid.insert(self.y, [self.erase_cell() for _ in range(self.cols)])
        elif final == "M":
            if self.top <= self.y <= self.bottom:
                n = min(arg(0, 1), self.bottom - self.y + 1)
                for _ in range(n):
                    del self.grid[self.y]
                    self.grid.insert(self.bottom, [self.erase_cell() for _ in range(self.cols)])
        elif final == "@":
            n = min(arg(0, 1), self.cols - self.x)
            row = self.grid[self.y]
            row[self.x:] = ([self.erase_cell() for _ in range(n)] + row[self.x:])[: self.cols - self.x]
        elif final == "P":
            n = min(arg(0, 1), self.cols - self.x)
            row = self.grid[self.y]
            row[self.x:] = row[self.x + n:] + [self.erase_cell() for _ in range(n)]
        elif final == "X":
            self.clear_range(self.y, self.x, min(self.cols, self.x + arg(0, 1)))
        elif final == "S":
            self.scroll_up(arg(0, 1))
        elif final == "T":
            self.scroll_down(arg(0, 1))
        elif final == "r":
            top, bottom = arg(0, 1) - 1, (nums[1] if len(nums) > 1 and nums[1] else self.rows) - 1
            if 0 <= top < bottom < self.rows:
                self.top, self.bottom = top, bottom
            self.x, self.y, self.pending = 0, 0, False
        elif final == "s":
            self.saved = (self.x, self.y, self.fg, self.bg, self.mod)
        elif final == "u":
            self.x, self.y, self.fg, self.bg, self.mod = self.saved
            self.x, self.y = min(self.x, self.cols - 1), min(self.y, self.rows - 1)
        elif final == "g":
            if (nums[0] if nums else 0) in (3, None): self.tabs.clear()
            elif (nums[0] if nums else 0) == 0: self.tabs.discard(self.x)

    def clear_range(self, y, x0, x1):
        if 0 <= y < self.rows:
            row = self.grid[y]
            for x in range(max(0, x0), min(self.cols, x1)):
                row[x] = self.erase_cell()

    def alt_screen(self, on, save_cursor):
        if on and self.alt is None:
            self.alt = (self.grid, self.x, self.y)
            self.grid = self.new_grid()
            if save_cursor: self.x = self.y = 0
        elif not on and self.alt is not None:
            self.grid, self.x, self.y = self.alt
            self.alt = None
            if len(self.grid) != self.rows or len(self.grid[0]) != self.cols:
                self.grid = [[self.blank() for _ in range(self.cols)] for _ in range(self.rows)]

    def sgr(self, body):
        parts = body.split(";") if body else ["0"]
        i = 0
        while i < len(parts):
            sub = parts[i].split(":")
            try:
                p = int(sub[0]) if sub[0] != "" else 0
            except ValueError:
                i += 1
                continue
            if p == 0:
                self.fg = self.bg = self.mod = 0
            elif p == 1: self.mod |= BOLD
            elif p == 2: self.mod |= DIM
            elif p == 3: self.mod |= ITALIC
            elif p == 4: self.mod |= UNDER
            elif p == 7: self.mod |= REVERSE
            elif p == 8: self.mod |= HIDDEN
            elif p == 9: self.mod |= STRIKE
            elif p == 21 or p == 22: self.mod &= ~(BOLD | DIM)
            elif p == 23: self.mod &= ~ITALIC
            elif p == 24: self.mod &= ~UNDER
            elif p == 27: self.mod &= ~REVERSE
            elif p == 28: self.mod &= ~HIDDEN
            elif p == 29: self.mod &= ~STRIKE
            elif 30 <= p <= 37: self.fg = (p - 30) + 1
            elif 90 <= p <= 97: self.fg = (p - 90) + 8 + 1
            elif 40 <= p <= 47: self.bg = (p - 40) + 1
            elif 100 <= p <= 107: self.bg = (p - 100) + 8 + 1
            elif p == 39: self.fg = 0
            elif p == 49: self.bg = 0
            elif p in (38, 48):
                color, used = self.extended(sub, parts, i)
                i += used
                if color is not None:
                    if p == 38: self.fg = color
                    else: self.bg = color
            i += 1

    @staticmethod
    def extended(sub, parts, i):
        """38/48 with ;5;n or ;2;r;g;b, or the colon form. Returns (color, extra parts consumed)."""
        try:
            if len(sub) > 1:  # 38:5:n or 38:2:[cs:]r:g:b
                nums = [int(x) if x else 0 for x in sub[1:]]
                if nums[0] == 5: return (1 << 24) | (nums[1] & 255), 0
                if nums[0] == 2:
                    r, g, b = nums[-3:]
                    return (2 << 24) | ((r & 255) << 16) | ((g & 255) << 8) | (b & 255), 0
                return None, 0
            mode = int(parts[i + 1])
            if mode == 5:
                return (1 << 24) | (int(parts[i + 2]) & 255), 2
            if mode == 2:
                r, g, b = int(parts[i + 2]), int(parts[i + 3]), int(parts[i + 4])
                return (2 << 24) | ((r & 255) << 16) | ((g & 255) << 8) | (b & 255), 4
        except (IndexError, ValueError):
            pass
        return None, 0

    # ---- reading back ------------------------------------------------------------------------
    def text_rows(self):
        return ["".join(c[0] for c in row).rstrip() for row in self.grid]
