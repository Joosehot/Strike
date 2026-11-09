"""Writes tests/uc/floatprint.c, .go and .py: the same doubles (edge cases,
then random bit patterns and random decimals, from a fixed seed) printed in
every format each language has, and their .out files from the real
toolchains (clang -O2, go run, py), CRLF made LF.

    py tools/float_values.py

Each value is written as a literal of 17 significant digits, so the text
printed is never the text read: the digits come from the double itself."""
import os, random, shutil, struct, subprocess, tempfile

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'tests', 'uc')
CLANG = r'C:\Program Files\LLVM\bin\clang.exe'
GO = r'C:\Program Files\Go\bin\go.exe'


def values():
    rnd = random.Random(20261109)
    vals = []
    seen = set()

    def add(x):
        b = struct.pack('<d', x)
        if x == x and abs(x) != float('inf') and b not in seen:
            seen.add(b)
            vals.append(x)

    for x in [0.0, -0.0, 0.5, 1.5, 2.5, -0.5, 0.125, 0.375, 1.005, 2.675, 0.1, 0.2, 0.3, 1 / 3, 2 / 3, 123456789.0,
              999999.5, 9999995.0, 0.000123456, 5e-324, 1e-323, 2.2250738585072014e-308, 2.225073858507201e-308,
              1.7976931348623157e308, 1e23, 9007199254740993.0, 4.35, 0.45, 8.5, 9.5, 0.0005, 0.00015, 1e21, 1e22,
              123.456, 1e15, 1e16, 1e17, 1e-4, 1e-5, 100000.0, 1e6, 1234567.0, 0.1 + 0.2, 3.4028234663852886e38]:
        add(x)
    for e in range(-1074, 1024, 37):
        add(2.0 ** e)
    for e in range(-320, 309, 23):
        add(float('1e%d' % e))
    while len(vals) < 260:
        add(struct.unpack('<d', struct.pack('<Q', rnd.getrandbits(64)))[0])
    while len(vals) < 330:
        add(rnd.uniform(-1e6, 1e6))
    while len(vals) < 400:
        add(round(rnd.uniform(-1000, 1000), rnd.randint(0, 6)))
    return vals


def lit(x):
    s = '%.17g' % x
    if 'e' not in s and '.' not in s and 'n' not in s:
        s += '.0'
    return s


def fits32(x):
    return abs(x) <= 3.4028234663852886e38


def write():
    vals = values()
    # C: printf's formats
    c = ['#include <stdio.h>', '', '// written by tools/float_values.py: the same doubles in every format', 'int main() {']
    for x in vals:
        c.append('    printf("%%.17g|%%f|%%e|%%g|%%.3f|%%.0e|%%.12g|%%.30e|%%E|%%.1g\\n", %s, %s, %s, %s, %s, %s, %s, %s, %s, %s);' % ((lit(x),) * 10))
    c += ['    printf("%f %e %g %f\\n", 1e308 * 10, -1e308 * 10, 1e308 * 10, -0.0);', '    return 0;', '}', '']
    # Go: Println's %v, and Printf's verbs; float32 where it fits
    g = ['package main', '', 'import "fmt"', '', '// written by tools/float_values.py: the same doubles in every format', 'func main() {']
    for x in vals:
        g.append('\tfmt.Println(%s)' % lit(x))
        g.append('\tfmt.Printf("%%e|%%.3f|%%g|%%.10g|%%G|%%.0f\\n", %s, %s, %s, %s, %s, %s)' % ((lit(x),) * 6))
        if fits32(x):
            g.append('\tfmt.Println(float32(%s))' % lit(x))
    g += ['\tvar z float64', '\tfmt.Println(1/z, -1/z, z/z, -z, []float64{%s, %s, %s})' % (lit(vals[3]), lit(vals[9]), lit(vals[20])), '}', '']
    # Python: repr
    p = ['# written by tools/float_values.py: the same doubles in every format']
    for x in vals:
        p.append('print(%s)' % lit(x))
    p.append('print([%s, %s, %s], 1e308 * 10, -1e308 * 10, 1e308 * 10 - 1e308 * 10)' % (lit(vals[3]), lit(vals[9]), lit(vals[20])))
    p.append('')
    for name, text in [('floatprint.c', c), ('floatprint.go', g), ('floatprint.py', p)]:
        with open(os.path.join(HERE, name), 'w', newline='\n') as f:
            f.write('\n'.join(text))


def real(name):
    path = os.path.join(HERE, name)
    d = tempfile.mkdtemp()
    try:
        if name.endswith('.c'):
            exe = os.path.join(d, 'a.exe')
            subprocess.run([CLANG, '-O2', '-w', path, '-o', exe], check=True)
            out = subprocess.run([exe], capture_output=True, check=True).stdout
        elif name.endswith('.go'):
            shutil.copy(path, os.path.join(d, 'main.go'))
            out = subprocess.run([GO, 'run', 'main.go'], cwd=d, capture_output=True, check=True).stdout
        else:
            out = subprocess.run(['py', path], capture_output=True, check=True).stdout
    finally:
        shutil.rmtree(d, ignore_errors=True)
    with open(path + '.out', 'wb') as f:
        f.write(out.replace(b'\r\n', b'\n'))


if __name__ == '__main__':
    write()
    for n in ['floatprint.c', 'floatprint.go', 'floatprint.py']:
        real(n)
        print(n, 'written')
