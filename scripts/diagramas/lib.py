"""Gerador de diagramas do Teko (tema Brasil/Amazônia) seguindo a skill diagram-design.

Gera, para cada diagrama: docs/diagramas/<nome>.svg, .html e .png (2x, via Brave headless).
Uso: python3 scripts/diagramas/gerar.py [nome ...]
Tokens e regras: docs/diagramas/TEMA.md
"""
import html, os, subprocess, shutil, tempfile

PAPER, PAPER2 = "#F5EFE3", "#EFE6D3"
INK, MUTED, SOFT = "#1A2B22", "#4A5C52", "#7C8A80"
GOLD, GOLD_TINT = "#F2B705", "#FFF4CC"
LINK, GREEN, RED = "#1F4BB8", "#12663B", "#C62F24"
ONCA, BOTO = "#9A5208", "#B8365F"
RULE = "#CFC5AE"
WHITE = "#FFFDF8"
SANS = "'Geist','Avenir Next','Segoe UI',system-ui,sans-serif"
MONO = "'Geist Mono',ui-monospace,Menlo,monospace"
SERIF = "'Instrument Serif',Georgia,serif"
FONTS = ("https://fonts.googleapis.com/css2?family=Instrument+Serif:ital@0;1&family="
         "Geist:wght@400;500;600&family=Geist+Mono:wght@400;500;600&display=swap")
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "docs", "diagramas")
BRAVE = "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"

ARROWS = {"default": MUTED, "accent": GOLD, "link": LINK, "ok": GREEN, "bad": RED, "onca": ONCA, "boto": BOTO}
# kind -> (fill, stroke, dash)
KINDS = {
    "plain": (WHITE, INK, None), "focal": (GOLD_TINT, GOLD, None),
    "store": ("#EAE3D2", MUTED, None), "external": ("#F0EADB", "#B9B09A", None),
    "input": ("#E9E4D6", SOFT, None), "optional": ("#F4EEE0", "#B9B09A", "4,3"),
    "security": ("#FBF1D6", "#B88A00", "4,4"), "onca": ("#FBEFDD", ONCA, None),
    "boto": ("#FBE9EF", BOTO, None), "ok": ("#E6F1EA", GREEN, None),
    "bad": ("#FBE8E6", RED, None), "link": ("#E8EEFA", LINK, None),
}
TAGCOL = {"focal": "#8A6700", "onca": ONCA, "boto": BOTO, "ok": GREEN, "bad": RED, "link": LINK}


def esc(s):
    return html.escape(str(s), quote=False)


class D:
    def __init__(self, nome, w, h, titulo, subtitulo="", eyebrow="TEKO"):
        self.nome, self.w, self.h = nome, w, h
        self.titulo, self.subtitulo, self.eyebrow = titulo, subtitulo, eyebrow
        self.under, self.over = [], []  # arrows first, nodes after
        self.top = 96 if subtitulo else 76
        self.legend_items = []

    # ---------- primitivos ----------
    def region(self, x, y, w, h, label, kind="optional"):
        fill, stroke, dash = KINDS[kind]
        self.under.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="{PAPER2}" '
                          f'fill-opacity="0.55" stroke="{stroke}" stroke-width="1" stroke-dasharray="5,4"/>')
        self.under.append(f'<text x="{x+12}" y="{y+16}" fill="{MUTED}" font-size="9" font-family="{MONO}" '
                          f'letter-spacing="0.08em">{esc(label.upper())}</text>')

    def node(self, x, y, w, h, nome, sub=None, kind="plain", tag=None, sub2=None):
        fill, stroke, dash = KINDS[kind]
        da = f' stroke-dasharray="{dash}"' if dash else ""
        o = self.over
        o.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="{PAPER}"/>')
        o.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="{fill}" stroke="{stroke}" stroke-width="1"{da}/>')
        cx = x + w / 2
        ty = y + h / 2
        if tag:
            tw = len(tag) * 5.6 + 12
            tc = TAGCOL.get(kind, MUTED)
            o.append(f'<rect x="{x+8}" y="{y+6}" width="{tw:.0f}" height="12" rx="2" fill="none" stroke="{tc}" '
                     f'stroke-opacity="0.5" stroke-width="0.8"/>')
            o.append(f'<text x="{x+8+tw/2:.0f}" y="{y+15}" fill="{tc}" font-size="7" font-family="{MONO}" '
                     f'text-anchor="middle" letter-spacing="0.08em">{esc(tag.upper())}</text>')
        lines = 1 + (1 if sub else 0) + (1 if sub2 else 0)
        base = ty - (lines - 1) * 8 + (4 if not tag else 8) - (0 if lines == 1 else 0)
        o.append(f'<text x="{cx}" y="{base:.0f}" fill="{INK}" font-size="12" font-weight="600" '
                 f'font-family="{SANS}" text-anchor="middle">{esc(nome)}</text>')
        yy = base + 15
        for s in (sub, sub2):
            if s:
                o.append(f'<text x="{cx}" y="{yy:.0f}" fill="{MUTED}" font-size="9" font-family="{MONO}" '
                         f'text-anchor="middle">{esc(s)}</text>')
                yy += 13

    def arrow(self, pts, kind="default", dashed=False, label=None, at=None, r=8, both=False):
        col = ARROWS[kind]
        d = f"M{pts[0][0]},{pts[0][1]}"
        for i in range(1, len(pts) - 1):
            (x0, y0), (x1, y1), (x2, y2) = pts[i - 1], pts[i], pts[i + 1]
            l1 = abs(x1 - x0) + abs(y1 - y0)
            l2 = abs(x2 - x1) + abs(y2 - y1)
            rr = min(r, l1 / 2, l2 / 2)
            ax = x1 - rr * _sgn(x1 - x0); ay = y1 - rr * _sgn(y1 - y0)
            bx = x1 + rr * _sgn(x2 - x1); by = y1 + rr * _sgn(y2 - y1)
            d += f" L{ax},{ay} Q{x1},{y1} {bx},{by}"
        d += f" L{pts[-1][0]},{pts[-1][1]}"
        da = ' stroke-dasharray="5,4"' if dashed else ""
        ms = f' marker-start="url(#a-{kind}-s)"' if both else ""
        self.under.append(f'<path d="{d}" fill="none" stroke="{col}" stroke-width="1.4"{da} '
                          f'marker-end="url(#a-{kind})"{ms}/>')
        if label:
            self.label(label, at or _mid(pts, label))

    def label(self, s, at, color=None):
        x, y = at
        w = len(s) * 5.0 + 10
        self.over.append(f'<rect x="{x-w/2:.0f}" y="{y-7}" width="{w:.0f}" height="14" rx="2" fill="{PAPER}"/>')
        self.over.append(f'<text x="{x}" y="{y+3}" fill="{color or MUTED}" font-size="8" font-family="{MONO}" '
                         f'text-anchor="middle" letter-spacing="0.05em">{esc(s)}</text>')

    def text(self, x, y, s, size=11, color=MUTED, anchor="start", mono=False, weight=400, italic=False):
        fam = MONO if mono else SANS
        st = ' font-style="italic"' if italic else ""
        self.over.append(f'<text x="{x}" y="{y}" fill="{color}" font-size="{size}" font-weight="{weight}" '
                         f'font-family="{fam}" text-anchor="{anchor}"{st}>{esc(s)}</text>')

    def legend(self, items):
        """items: (cor_traco, rotulo, tracejado?)"""
        self.legend_items = items

    # ---------- sequência ----------
    def sequence(self, atores, mensagens, y0=None, passo=40, notas=()):
        """atores: [(chave, nome, sub, kind, x)]; mensagens: [(de, para, rotulo, kind, tracejado)];
        notas: [(indice_mensagem_apos, chave_ator, texto, largura)] desenhadas abaixo da mensagem."""
        y0 = y0 or self.top + 8
        nh = 44
        xs = {}
        for k, n, sub, kind, x in atores:
            xs[k] = x
        ymsg = y0 + nh + 28
        total = len(mensagens) * passo + sum(40 for _ in notas) + 20
        yend = ymsg + total
        for k, n, sub, kind, x in atores:
            self.under.append(f'<line x1="{x}" y1="{y0+nh}" x2="{x}" y2="{yend}" stroke="{RULE}" '
                              f'stroke-width="1" stroke-dasharray="3,4"/>')
        for k, n, sub, kind, x in atores:
            self.node(x - 70, y0, 140, nh, n, sub, kind)
        y = ymsg
        notas_por = {}
        for i, k, t, w in notas:
            notas_por.setdefault(i, []).append((k, t, w))
        for i, (de, para, rot, kind, tr) in enumerate(mensagens):
            x1, x2 = xs[de], xs[para]
            if de == para:
                self.arrow([(x1, y), (x1 + 36, y), (x1 + 36, y + 14), (x1 + 2, y + 14)], kind, tr)
                self.text(x1 + 44, y + 10, rot, 9, MUTED, mono=True)
            else:
                s = 1 if x2 > x1 else -1
                self.arrow([(x1 + 3 * s, y), (x2 - 3 * s, y)], kind, tr)
                self.label(rot, ((x1 + x2) / 2, y - 12))
            y += passo
            for k, t, w in notas_por.get(i, []):
                self.note(xs[k] - w / 2, y - 14, w, t)
                y += 40
        self.seq_end = yend
        self.h = int(yend + 80)
        return yend

    def note(self, x, y, w, texto):
        x = max(16, min(x, self.w - 16 - w))
        self.over.append(f'<rect x="{x:.0f}" y="{y}" width="{w}" height="26" rx="4" fill="{GOLD_TINT}" stroke="{GOLD}" '
                         f'stroke-width="1"/>')
        self.over.append(f'<text x="{x+w/2:.0f}" y="{y+16}" fill="{INK}" font-size="9.5" font-family="{SANS}" '
                         f'text-anchor="middle">{esc(texto)}</text>')

    # ---------- saída ----------
    def svg(self):
        W, H = self.w, self.h
        leg_y = H - 36
        marks = "".join(
            f'<marker id="a-{k}" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto"><polygon points="0 0,8 3,0 6" fill="{c}"/></marker>'
            f'<marker id="a-{k}-s" markerWidth="8" markerHeight="6" refX="1" refY="3" orient="auto"><polygon points="8 0,0 3,8 6" fill="{c}"/></marker>'
            for k, c in ARROWS.items())
        head = (f'<text x="32" y="30" fill="{MUTED}" font-size="9" font-family="{MONO}" letter-spacing="0.14em">'
                f'{esc(self.eyebrow.upper())}</text>'
                f'<text x="32" y="62" fill="{INK}" font-size="28" font-family="{SERIF}">{esc(self.titulo)}</text>')
        if self.subtitulo:
            head += (f'<text x="32" y="82" fill="{MUTED}" font-size="12" font-family="{SANS}">{esc(self.subtitulo)}</text>')
        # faixa tricolor discreta (verde, ouro, azul) no topo — identidade Brasil
        band = (f'<rect x="0" y="0" width="{W*0.5:.0f}" height="4" fill="{GREEN}"/>'
                f'<rect x="{W*0.5:.0f}" y="0" width="{W*0.3:.0f}" height="4" fill="{GOLD}"/>'
                f'<rect x="{W*0.8:.0f}" y="0" width="{W*0.2:.0f}" height="4" fill="{LINK}"/>')
        leg = f'<line x1="32" y1="{leg_y-10}" x2="{W-32}" y2="{leg_y-10}" stroke="{RULE}" stroke-width="1"/>'
        x = 32
        for col, rot, dash in self.legend_items:
            da = ' stroke-dasharray="5,4"' if dash else ""
            leg += f'<line x1="{x}" y1="{leg_y+6}" x2="{x+22}" y2="{leg_y+6}" stroke="{col}" stroke-width="1.6"{da}/>'
            leg += f'<text x="{x+30}" y="{leg_y+10}" fill="{MUTED}" font-size="10" font-family="{SANS}">{esc(rot)}</text>'
            x += 30 + len(rot) * 5.6 + 26
        leg += (f'<text x="{W-32}" y="{leg_y+10}" fill="{SOFT}" font-size="9" font-family="{MONO}" text-anchor="end">'
                f'teko · protótipo de estudo</text>')
        style = f'<style>@import url("{FONTS}");</style>'
        return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
                f'role="img" aria-label="{esc(self.titulo)}">{style}<defs>{marks}</defs>'
                f'<rect width="100%" height="100%" fill="{PAPER}"/>{band}{head}'
                f'{"".join(self.under)}{"".join(self.over)}{leg}</svg>')

    def salvar(self):
        os.makedirs(OUT, exist_ok=True)
        svg = self.svg()
        base = os.path.join(OUT, self.nome)
        open(base + ".svg", "w", encoding="utf-8").write(svg)
        page = (f'<!doctype html><html lang="pt-BR"><head><meta charset="utf-8"><meta name="viewport" '
                f'content="width=device-width,initial-scale=1"><title>{esc(self.titulo)}</title>'
                f'<link href="{FONTS}" rel="stylesheet"><style>:root{{--color-paper:{PAPER};--color-ink:{INK};'
                f'--color-muted:{MUTED};--color-accent:{GOLD};--color-link:{LINK}}}'
                f'body{{margin:0;background:var(--color-paper);display:flex;justify-content:center}}'
                f'svg{{max-width:100%;height:auto}}</style></head><body>{svg}</body></html>')
        open(base + ".html", "w", encoding="utf-8").write(page)
        _png(svg, base + ".png", self.w, self.h)
        return base


def _png(svg, dest, w, h):
    with tempfile.TemporaryDirectory() as td:
        p = os.path.join(td, "i.html")
        open(p, "w", encoding="utf-8").write(
            f'<!doctype html><meta charset="utf-8"><body style="margin:0;background:{PAPER}">{svg}</body>')
        subprocess.run([BRAVE, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                        "--force-device-scale-factor=2", f"--window-size={w},{h}",
                        "--virtual-time-budget=10000", f"--screenshot={dest}", "file://" + p],
                       check=True, capture_output=True)


def _sgn(v):
    return (v > 0) - (v < 0)


def _mid(pts, label=""):
    # meio do segmento mais longo; horizontal: 14px acima; vertical: ao lado da linha
    best = max(range(len(pts) - 1), key=lambda i: abs(pts[i+1][0]-pts[i][0]) + abs(pts[i+1][1]-pts[i][1]))
    a, b = pts[best], pts[best + 1]
    if a[1] == b[1]:
        return ((a[0] + b[0]) / 2, a[1] - 14)
    return (a[0] + len(label) * 2.5 + 16, (a[1] + b[1]) / 2)
