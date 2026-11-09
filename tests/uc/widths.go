package main

// Go's int32 wraps at 32 bits, int at 64

import "fmt"

func add(a, b int32) int32 {
	return a + b
}

func main() {
	var x int32 = 2147483647
	x = x + 1
	fmt.Println(x)
	fmt.Println(add(2000000000, 2000000000))
	var y int = 2147483647
	y = y + 1
	fmt.Println(y)
}

