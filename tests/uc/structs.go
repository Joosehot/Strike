package main

import (
	"fmt"
	"strconv"
)

// Go's structs: values (copied when assigned, given, returned, and their
// arrays with them), pointers to them on the heap, methods with value and
// pointer receivers, printed as fmt prints them.

type Inner struct {
	A, B int
}

type P struct {
	X, Y int
	Name string
	In   Inner
	Arr  [3]int
	F    float64
	Ok   bool
	S    []int
}

type Counter struct {
	n int
}

type Node struct {
	V    int
	Next *Node
}

type Vec struct {
	X, Y float64
}

type Shape struct {
	Name   string
	Pts    [2]Vec
	Tags   []string
	Scale  float32
	Hidden bool
	ID     int32
	Kids   []Inner
}

type Money struct {
	Cents int
}

var origin = Vec{0, 0}
var registry []Shape
var keep []Vec
var zero P

func (c *Counter) Inc(by int) {
	c.n += by
}

func (c Counter) Get() int {
	return c.n
}

func (p P) Sum() int {
	return p.X + p.Y + p.In.A + p.Arr[0]
}

func (p *P) Move(dx int) {
	p.X += dx
	p.Arr[0] = dx
}

func (v Vec) Add(o Vec) Vec {
	return Vec{v.X + o.X, v.Y + o.Y}
}

func (v *Vec) Scale(k float64) *Vec {
	v.X *= k
	v.Y *= k
	return v
}

func (s Shape) Moved(d Vec) Shape {
	s.Pts[0] = s.Pts[0].Add(d)
	s.Pts[1] = s.Pts[1].Add(d)
	return s
}

func (m Money) String() string {
	return "$" + strconv.Itoa(m.Cents/100) + "." + strconv.Itoa(m.Cents%100)
}

func mk(x int) P {
	return P{X: x, Y: x * 2, Name: "mk"}
}

func change(p P) int {
	p.X = 100
	p.Arr[1] = 7
	return p.X
}

func newCounter() *Counter {
	return &Counter{n: 10}
}

func push(n *Node, v int) *Node {
	return &Node{V: v, Next: n}
}

func length(n *Node, max int) int {
	c := 0
	for i := 0; i < max; i++ {
		c += n.V
		n = n.Next
	}
	return c
}

func register(s Shape) {
	registry = append(registry, s)
}

func main() {
	var z P
	fmt.Println(z, zero.Name == "", len(zero.S))
	p := P{1, 2, "hi", Inner{3, 4}, [3]int{5, 6, 7}, 1.5, true, []int{8, 9}}
	fmt.Println(p)
	fmt.Printf("%v\n", p)
	fmt.Printf("%+v\n", p)
	q := p
	q.X = 10
	q.Arr[0] = 50
	q.In.A = 30
	fmt.Println(p.X, p.Arr, p.In, q.X, q.Arr, q.In)
	fmt.Println(change(p), p.X, p.Arr)
	fmt.Println(p.Sum(), mk(3).Sum())
	p.Move(4)
	fmt.Println(p.X, p.Arr[0])
	pp := &p
	pp.Move(1)
	pp.Y = 77
	fmt.Println(p.X, p.Y, pp.Sum())
	c := Counter{}
	c.Inc(3)
	c.Inc(4)
	fmt.Println(c.Get(), c)
	cp := newCounter()
	cp.Inc(5)
	fmt.Println(cp.Get(), cp)
	m := mk(6)
	fmt.Println(m)
	ps := []P{mk(1), mk(2)}
	ps = append(ps, mk(3))
	ps[1].X = 20
	for i := range ps {
		ps[i].Move(i)
	}
	for _, v := range ps {
		v.X = 1000
	}
	fmt.Println(len(ps), ps[0].X, ps[1].X, ps[2].X)
	ins := []Inner{{1, 2}, {3, 4}}
	fmt.Println(ins)
	fmt.Printf("%+v\n", ins)
	var arr [2]Inner
	arr[1].B = 5
	fmt.Println(arr)
	ptrs := []*Counter{}
	for i := 0; i < 3; i++ {
		k := Counter{i}
		ptrs = append(ptrs, &k)
	}
	fmt.Println(ptrs[0].n, ptrs[1].n, ptrs[2].n)
	var np *Counter
	fmt.Println(np)

	var head *Node
	for i := 1; i <= 5; i++ {
		head = push(head, i*i)
	}
	fmt.Println(length(head, 5), head.V, head.Next.Next.V)
	head.Next.V = 100
	fmt.Println(length(head, 3))

	a := Vec{1, 2}
	b := a.Add(Vec{0.5, 0.25})
	fmt.Println(a, b, origin)
	a.Scale(2).Scale(3)
	fmt.Println(a)
	pa := &a
	cc := *pa
	cc.X = -1
	*pa = Vec{9, 8}
	fmt.Println(a, cc, *pa, (*pa).Y)

	s := Shape{Name: "sq", Pts: [2]Vec{{1, 1}, {2, 2}}, Tags: []string{"a", "b"}, Scale: 0.1, ID: 7, Kids: []Inner{{1, 1}}}
	t := s.Moved(Vec{10, 20})
	fmt.Println(s)
	fmt.Println(t)
	fmt.Printf("%+v\n", t)
	t.Tags[0] = "shared"
	t.Kids[0].A = 2
	fmt.Println(s.Tags, t.Tags, s.Kids)
	register(s)
	register(t)
	registry[0].Name = "first"
	fmt.Println(len(registry), registry[0].Name, s.Name, registry[1].Pts[1].X)

	vs3 := [3]Vec{{1, 2}, {3, 4}}
	brr := vs3
	brr[0].X = 100
	fmt.Println(vs3, brr)
	for i, v := range vs3 {
		v.Y = 50
		vs3[i].X += 1
		fmt.Println(i, v)
	}
	fmt.Println(vs3)
	pv := &vs3[1]
	pv.Scale(10)
	fmt.Println(vs3[1], *pv)
	pin := &s.Pts[0]
	pin.X = 42
	fmt.Println(s.Pts)

	var vs []Vec
	caps := []int{}
	for i := 0; i < 40; i++ {
		before := cap(vs)
		vs = append(vs, Vec{float64(i), 0})
		if cap(vs) != before {
			caps = append(caps, cap(vs))
		}
	}
	keep = vs
	fmt.Println(caps, len(vs), vs[39], len(keep))
	var shapes []Shape
	scaps := []int{}
	for i := 0; i < 20; i++ {
		before := cap(shapes)
		shapes = append(shapes, Shape{ID: int32(i)})
		if cap(shapes) != before {
			scaps = append(scaps, cap(shapes))
		}
	}
	registry = shapes
	fmt.Println(scaps, shapes[19].ID, shapes[3].Name == "")
	ms := make([]Shape, 2)
	fmt.Println(ms)

	money := Money{1234}
	var nm *Money
	fmt.Println(money, &money, nm)
	fmt.Printf("%v %+v\n", money, money)
}
