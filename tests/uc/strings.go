// Strings as Go makes them: bytes, s[i] a byte, range over the code
// points, comparisons, the fmt verbs, strconv.Itoa, string(r).
package main

import (
	"fmt"
	"strconv"
)

var greeting string = "hey"
var nothing string
var shelf [3]string

func shout(s string) string {
	return s + "!"
}

func reverse(s string) string {
	out := ""
	for i := len(s) - 1; i >= 0; i-- {
		out += string(rune(s[i]))
	}
	return out
}

func count(s string, c byte) int {
	n := 0
	for i := 0; i < len(s); i++ {
		if s[i] == c {
			n++
		}
	}
	return n
}

func next(b byte) byte {
	return b + 1
}

func digits(n int) []int {
	s := strconv.Itoa(n)
	out := []int{}
	for i := 0; i < len(s); i++ {
		out = append(out, int(s[i]-'0'))
	}
	return out
}

func main() {
	s := "hello"
	t := "wörld"
	fmt.Println(s, t, len(s), len(t), greeting, nothing == "", len(nothing))
	fmt.Println(s[0], s[4], t[1], t[2], s[1:3], s[:2], s[3:], s[5:], s[2:2] == "")
	u := s + ", " + t
	fmt.Println(u, len(u))
	fmt.Println(s == "hello", s != t, s < t, "abc" < "abd", "ab" < "abc", "b" >= "abc", "" < "a", "Z" < "a")
	fmt.Printf("%s|%v|%d|%c%c%c|%s\n", s, t, len(u), 'Z', 233, 0x1F600, "lit")
	ctl := "bell" + string(rune(7)) + " back" + string(rune(8)) + " feed" + string(rune(12)) + " vt" + string(rune(11)) + " ret\r nul" + string(rune(0)) + string(rune(31)) + string(rune(127))
	fmt.Printf("%q %q %q\n", "a\"b\\c\n\t", "", ctl)
	fmt.Println(strconv.Itoa(42), strconv.Itoa(-9), strconv.Itoa(0), strconv.Itoa(-9223372036854775807-1))
	for i, c := range t {
		fmt.Println(i, c, string(c))
	}
	for i := range "ab" {
		fmt.Println(i)
	}
	// bytes that are not UTF-8: a lone first byte, a sequence cut short
	runes := ""
	for i, c := range t[1:2] + "é" + "€"[:2] + "x" {
		runes += strconv.Itoa(i) + ":" + strconv.Itoa(int(c)) + " "
	}
	fmt.Println(runes)
	words := []string{"a", "bc", ""}
	words = append(words, "d e")
	fmt.Println(words, len(words), cap(words))
	fmt.Printf("%v %s\n", words, words)
	more := make([]string, 2, 3)
	more[1] = "x"
	more = append(more, "y", "z")
	fmt.Println(more, len(more[0]), cap(more))
	shelf[1] = "mid"
	fmt.Println(shelf, len(shelf[0]))
	fmt.Println(shout(s), reverse("drawer"), reverse(""), count("banana", 'a'))
	acc := ""
	for i := 0; i < 5; i++ {
		acc += strconv.Itoa(i * i)
	}
	fmt.Println(acc, len(acc))
	var b byte = s[1]
	b = b + 200
	fmt.Println(b, b+100, ^b, -b, b/3, b>>1, b<<1, b*3, next(255), next(b))
	c := s[0] - 'a'
	c -= 10
	fmt.Println(c, int(c)+1, string(rune(s[0])), string(rune(c)), len(string(rune(c))))
	fmt.Println(digits(9075), digits(0))
	code := ""
	for _, r := range "abc xyz" {
		if r >= 'a' && r <= 'z' {
			r = (r-'a'+3)%26 + 'a'
		}
		code += string(r)
	}
	fmt.Println(code)
	var grid [2][2]string
	grid[0][1] = "o"
	grid[1][0] = "x"
	fmt.Println(grid, len(grid[0][0]))
	line := "a"
	for len(line) < 20 {
		line = line + line
	}
	fmt.Println(line, len(line))
}
