package calc

var Scale = 10
var Total int
var count int

func Add(a, b int) int {
	count++
	return a + b
}

func Square(x int) int {
	count++
	return mul(x, x)
}
