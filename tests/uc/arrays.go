package main

import "fmt"

var board [3][3]int
var seen []int

func total(s []int) int {
	t := 0
	for _, v := range s {
		t += v
	}
	return t
}

func double(a [4]int) [4]int {
	for i := range a {
		a[i] *= 2
	}
	return a
}

func fill(s []int) {
	for i := 0; i < len(s); i++ {
		s[i] = i * i
	}
}

func record(x int) {
	seen = append(seen, x)
}

func main() {
	var s []int
	fmt.Println(len(s), cap(s), s)
	for i := 0; i < 20; i++ {
		s = append(s, i)
	}
	fmt.Println(len(s), cap(s), total(s))

	caps := []int{}
	var t []int
	for i := 0; i < 1100; i++ {
		before := cap(t)
		t = append(t, i)
		if cap(t) != before {
			caps = append(caps, cap(t))
		}
	}
	// t escapes (it is printed): Go keeps its elements on the heap, where
	// they grow by the allocator's size classes
	fmt.Println(caps, t[:3])

	var narrow []int32
	for i := 0; i < 9; i++ {
		narrow = append(narrow, int32(i))
	}
	fmt.Println(len(narrow), cap(narrow))

	var flags []bool
	for i := 0; i < 20; i++ {
		flags = append(flags, i%3 == 0)
	}
	fmt.Println(cap(flags), flags[:5])

	m := make([]int, 5, 8)
	fill(m)
	fmt.Println(m, len(m), cap(m))
	sub := m[1:3]
	sub[0] = 100
	fmt.Println(m, sub, len(sub), cap(sub))
	shared := append(m[:2], 77)
	fmt.Println(m, shared)

	a := [4]int{1, 2, 3, 4}
	b := a
	b[0] = 99
	d := double(a)
	fmt.Println(a, b, d)

	grid := make([][]int, 3)
	for i := range grid {
		grid[i] = make([]int, i+1)
		for j := range grid[i] {
			grid[i][j] = i + j
		}
	}
	fmt.Println(grid, len(grid[2]))

	lit := [][]int{{1, 2}, {3}, {}}
	lit[1] = append(lit[1], 4, 5)
	fmt.Println(lit)

	board[1][1] = 5
	board[0][2] = 7
	row := board[1]
	row[0] = 9
	fmt.Println(board, row)

	for i := 0; i < 5; i++ {
		record(i * 10)
	}
	fmt.Println(seen, len(seen))

	var w []int32
	w = append(w, 2147483647)
	w[0] += 1
	fmt.Println(w)

	sum := 0
	for i, v := range []int{10, 20, 30} {
		sum += i * v
	}
	fmt.Println(sum)

	primes := make([]bool, 100)
	count := 0
	for p := 2; p < 100; p++ {
		if !primes[p] {
			count++
			for q := p * p; q < 100; q += p {
				primes[q] = true
			}
		}
	}
	fmt.Println(count)
}
