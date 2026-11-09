package main

var counter = 41 + 1

// a struct and its method in one file of the package, used in another
type Tally struct {
	Name string
	N    int
}

func (t *Tally) Add(k int) {
	t.N += k
}

func twice(x int) int {
	return x * 2
}
