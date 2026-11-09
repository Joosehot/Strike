package main

// how many numbers, then the numbers: their sum

import "fmt"

func main() {
	n := 0
	sum := 0
	fmt.Scan(&n)
	for i := 0; i < n; i++ {
		x := 0
		fmt.Scan(&x)
		sum += x
	}
	fmt.Println(sum)
}