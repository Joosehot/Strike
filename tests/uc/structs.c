#include <stdio.h>
#include <stddef.h>
#include <string.h>

/* C's structs: laid out as clang lays them out, held in place, copied
   when assigned, given and returned; pointers to them, -> and &. */

struct Q {
    int a;
    char b;
};

struct P {
    int x;
    long long y;
    double z;
    char name[8];
    struct Q q;
    int a[3];
};

typedef struct {
    short s;
    long long t;
} T;

typedef struct Node Node;
struct Node {
    int v;
    Node *next;
};

typedef struct Vec {
    double x, y;
} Vec;

struct Rec {
    char c;
    short s;
    long l;
    long long ll;
    const char *label;
    Vec pts[2];
};

struct Grid {
    struct Q cells[2][3];
    int n;
};

struct P make(int x) {
    struct P p = {x, 2 * x, 1.5, "hi", {7, 'c'}, {1, 2, 3}};
    return p;
}

int sum(struct P p) {
    p.x += 100;
    p.a[0] = 0;
    return p.x + p.a[1] + p.a[2] + p.q.a;
}

void move(struct P *p, int dx) {
    p->x += dx;
    p->q.a = dx;
    p->a[2] *= 10;
}

Vec add(Vec a, Vec b) {
    Vec r = {a.x + b.x, a.y + b.y};
    return r;
}

Vec scale(Vec a, double k) {
    a.x *= k;
    a.y *= k;
    return a;
}

/* a struct returned through recursion: each call's own */
Vec sum_to(int n) {
    if (n == 0) {
        Vec z = {0, 0};
        return z;
    }
    Vec v = {n, -n};
    return add(sum_to(n - 1), v);
}

int total(Node *n, int count) {
    int t = 0;
    for (int i = 0; i < count; i++) {
        t += n->v;
        n = n->next;
    }
    return t;
}

void fill(struct Rec *rs, int n) {
    for (int i = 0; i < n; i++) {
        rs[i].c = 100 + 50 * i;
        rs[i].s = 30000 + 2000 * i;
        rs[i].l = 2147483647;
        rs[i].l += i;
        rs[i].ll = rs[i].l * 3;
        rs[i].pts[1].y = i * 0.5;
    }
}

/* a bubble sort that swaps whole structs */
void sort(struct Q *qs, int n) {
    for (int i = 0; i < n; i++) {
        for (int j = 0; j + 1 < n - i; j++) {
            if (qs[j].a > qs[j + 1].a) {
                struct Q t = qs[j];
                qs[j] = qs[j + 1];
                qs[j + 1] = t;
            }
        }
    }
}

int counter(void) {
    static struct Rec seen = {1, 2};
    seen.c++;
    return seen.c;
}

struct P g = {.y = 9, .x = 4};
struct Rec grecs[3];
struct Q gq[4] = {{5, 'a'}, {.b = 'b', .a = 3}, {9, 'e'}, {1, 'd'}};
T gt = {-1};

int main() {
    printf("%zu %zu %zu %zu %zu %zu\n", sizeof(struct P), sizeof(struct Q), sizeof(T), sizeof(Node), sizeof(struct Rec), sizeof(struct Grid));
    printf("%zu %zu %zu %zu\n", offsetof(struct P, y), offsetof(struct P, name), offsetof(struct P, q), offsetof(struct P, a));
    printf("%zu %zu %zu\n", offsetof(struct Rec, label), offsetof(struct Rec, pts), offsetof(struct Grid, n));

    struct P p = make(5);
    printf("%d %lld %f %s %d %c %d\n", p.x, p.y, p.z, p.name, p.q.a, p.q.b, p.a[2]);
    struct P r = p;
    r.x = 1;
    r.a[0] = 50;
    r.name[0] = 'H';
    printf("%d %d %d %d %s %s\n", p.x, r.x, p.a[0], r.a[0], p.name, r.name);
    printf("%d %d %d\n", sum(p), p.x, p.a[0]);
    move(&p, 3);
    printf("%d %d %d\n", p.x, p.q.a, p.a[2]);
    printf("%d %lld %d %d\n", make(8).x, make(8).y + make(1).y, make(2).a[1], sum(make(3)));
    printf("%d %lld %d %d\n", g.x, g.y, g.a[1], g.q.b);

    struct P ps[3] = {{1}, {2, 3}, {.z = 2.5}};
    ps[1].q.b = 'z';
    for (int i = 0; i < 3; i++) {
        printf("%d %lld %g %d\n", ps[i].x, ps[i].y, ps[i].z, ps[i].q.b);
    }
    ps[0] = ps[1];
    ps[1].y = -1;
    printf("%d %lld %lld\n", ps[0].x, ps[0].y, ps[1].y);

    T t = {3, 4};
    T *tp = &t;
    tp->t *= 10;
    tp->s--;
    printf("%d %lld %zu %d\n", t.s, t.t, sizeof t, gt.s);

    Node nodes[4];
    for (int i = 0; i < 4; i++) {
        nodes[i].v = (i + 1) * 10;
        nodes[i].next = &nodes[(i + 1) % 4];
    }
    printf("%d %d\n", total(&nodes[0], 4), total(nodes[2].next, 6));
    Node *np = &nodes[1];
    np->next->v = 5;
    (*np).v = 7;
    printf("%d %d %d\n", nodes[1].v, nodes[2].v, np->next->next->v);

    Vec a = {1.5, 2};
    Vec b = scale(a, 3);
    Vec c = add(a, add(b, b));
    printf("%g %g %g %g %g %g\n", a.x, a.y, b.x, b.y, c.x, c.y);
    printf("%g %g\n", sum_to(10).x, sum_to(4).y);
    a = add(a, a);
    printf("%g %g\n", a.x, a.y);
    Vec *vp = &b;
    *vp = c;
    Vec d = *vp;
    d.x = 0;
    printf("%g %g %g\n", b.x, d.x, d.y);

    fill(grecs, 3);
    for (int i = 0; i < 3; i++) {
        printf("%d %d %ld %lld %g\n", grecs[i].c, grecs[i].s, grecs[i].l, grecs[i].ll, grecs[i].pts[1].y);
    }
    struct Rec rec = {'a', -5, 6, 7, "label", {{1, 2}, {3, 4}}};
    struct Rec rec2 = {.label = "two", .s = 9, 10, .c = 'z'};
    printf("%c %d %ld %lld %s %g %g\n", rec.c, rec.s, rec.l, rec.ll, rec.label, rec.pts[0].y, rec.pts[1].x);
    printf("%c %d %ld %lld %s %zu\n", rec2.c, rec2.s, rec2.l, rec2.ll, rec2.label, strlen(rec2.label));
    rec.pts[0] = rec.pts[1];
    rec.pts[1].x += 100;
    printf("%g %g\n", rec.pts[0].x, rec.pts[1].x);

    int c1 = counter();
    int c2 = counter();
    printf("%d %d %d\n", c1, c2, counter());

    sort(gq, 4);
    for (int i = 0; i < 4; i++) {
        printf("%d%c ", gq[i].a, gq[i].b);
    }
    printf("\n");

    struct Grid grid = {{{{1, 'a'}, {2}}, {{3}}}, 6};
    struct Grid other = grid;
    other.cells[1][2].a = 99;
    grid.cells[0][1].b = 'q';
    printf("%d %d %d %d %c %d %d\n", grid.cells[0][0].a, grid.cells[0][1].a, grid.cells[1][0].a, grid.n, grid.cells[0][1].b, grid.cells[1][2].a, other.cells[1][2].a);
    printf("%zu %zu %d\n", sizeof(ps) / sizeof(ps[0]), sizeof p.name, (int)(sizeof(grid.cells) / sizeof(grid.cells[0][0])));

    /* a char pointer field points at an array, which it sees change */
    char buf[8] = "abc";
    struct Rec lr = {'l', 1, 2, 3, buf};
    buf[0] = 'x';
    printf("%s %c\n", lr.label, lr.label[2]);
    lr.label = "lit";
    grecs[1].label = buf;
    printf("%s %s %zu\n", lr.label, grecs[1].label, strlen(grecs[1].label));
    return 0;
}
