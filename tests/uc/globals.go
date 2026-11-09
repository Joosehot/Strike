package main

// globals: given their first values before main, changed by functions

import "fmt"

var counter = 10
var total int
var limit = 3 + 4

func bump(by int) {
	counter = counter + by
}

func nextID() int {
	counter++
	return counter
}

func main() {
	bump(5)
	fmt.Println(counter)
	a := nextID()
	b := nextID()
	fmt.Println(a, b)
	for i := 0; i < limit; i++ {
		total += i * counter
	}
	fmt.Println(total)
	counter := 99
	fmt.Println(counter)
}