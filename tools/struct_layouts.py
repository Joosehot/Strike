"""Writes tests/uc/layouts.c: 160 structs of random fields (every integer
width, double, pointers, char and int arrays, structs and arrays of
structs declared before, pointers to structs), from a fixed seed; main
prints each one's sizeof and every field's offsetof. Its .out file is what
clang -O2 printed, CRLF made LF.

    py tools/struct_layouts.py

Strike lays the structs out itself, and its executable must print what
clang's does."""
import os, random, subprocess, tempfile

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'tests', 'uc')
CLANG = r'C:\Program Files\LLVM\bin\clang.exe'

SCALARS = ['char', 'short', 'int', 'long', 'long long', 'double', 'int32_t', 'int64_t', 'char *', 'signed char', 'short int', 'long int']


def program():
    rnd = random.Random(20261109)
    out = ['#include <stdio.h>', '#include <stddef.h>', '#include <stdint.h>', '']
    structs = []
    main = []
    for k in range(160):
        name = f'S{k}'
        fields = []
        for i in range(1 + rnd.randrange(8)):
            f = f'f{i}'
            r = rnd.random()
            if r < 0.5 or not structs:
                t = rnd.choice(SCALARS)
                if rnd.random() < 0.25 and t != 'char *':
                    fields.append(f'{t} {f}[{1 + rnd.randrange(7)}]')
                else:
                    fields.append(f'{t} {f}')
            elif r < 0.75:
                fields.append(f'struct {rnd.choice(structs)} {f}')
            elif r < 0.9:
                fields.append(f'struct {rnd.choice(structs)} {f}[{1 + rnd.randrange(3)}]')
            else:
                # a pointer to any struct, this one too
                fields.append(f'struct {rnd.choice(structs + [name])} *{f}')
        # two fields of one type in one declaration, now and then
        if rnd.random() < 0.3:
            fields.append(f'{rnd.choice(["char", "short", "int", "long long"])} g0, g1[2]')
        out.append(f'struct {name} {{\n' + ''.join(f'    {x};\n' for x in fields) + '};')
        names = [x.split()[-1].lstrip('*').split('[')[0] for x in fields]
        if fields[-1].endswith('g1[2]'):
            names[-1:] = ['g0', 'g1']
        offs = ', '.join(f'offsetof(struct {name}, {n})' for n in names)
        fmt = ' '.join(['%zu'] * (len(names) + 1))
        main.append(f'    printf("{name}: {fmt}\\n", sizeof(struct {name}), {offs});')
        structs.append(name)
    out.append('')
    out.append('int main() {')
    out.extend(main)
    out.append('    return 0;')
    out.append('}')
    return '\n'.join(out) + '\n'


def main():
    src = os.path.join(HERE, 'layouts.c')
    with open(src, 'w', newline='\n') as f:
        f.write(program())
    with tempfile.TemporaryDirectory() as d:
        exe = os.path.join(d, 'layouts.exe')
        subprocess.run([CLANG, '-O2', src, '-o', exe], check=True)
        out = subprocess.run([exe], check=True, capture_output=True).stdout.replace(b'\r\n', b'\n')
    with open(src + '.out', 'wb') as f:
        f.write(out)


if __name__ == '__main__':
    main()
