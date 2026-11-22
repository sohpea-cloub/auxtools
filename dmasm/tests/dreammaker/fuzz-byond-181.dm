// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 181, "515.1647: new: another runtime error: type mismatch: "…" %%= N, not type mismatch: "…" %%= N"
#define DEBUG

#define FZD1 "a b"

var/fz_news = 0
var/fz_budget = 0
var/list/fz_out = list()
var/list/fz_logs = list()
var/list/fz_old = list()
/proc/fz_show(v, depth = 0)
/proc/fz_log(tag, v)
/proc/fz_line(t)
/proc/fz_sort(list/l)
/proc/fz_call(id, dom, datum/d, name)
/proc/fz_case(id, t, name, old)
/proc/fz_ti(t, list/names, list/procs, datum/old)
/proc/fz_main()
/world/New()

/datum/fz
	var/static/vt8 = "Hello"
	var/const/vn13 = -3.25

/datum/fz/s5

var/gn23

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(58, /obj/fz/s6, "t56", 1)
	fz_case(78, /obj/fz/s6, "t73", 1)

/proc/gh34(x1 = "Hello", x2, x3 = 16777215)
	return global.gn23

/datum/fz/s4/proc/h43(x1, x2 = 5, x3 = 2.5)
	if (list("Hello", num2text(vn13), ("abc" > vt8))[1])
		x2 %%= 10

/obj/fz/s6/proc/t56(a, b = 2, c)
	var/list/M = list("a" = 10, "b" = "q\"q")
	gn23 = max(M)

/obj/fz/s6/proc/t73(a, b = 2, c)
	var/n1 = 0.5
	var/list/M = list("a" = 16777215, "b" = "Hello")
	var/datum/fz/s4/p = new(2.5, "UPPER lower")
	M["a"] = p.h43(/datum/fz/s5::vn13, gh34(arglist(list((c || FZD1), (2147483647 + 0.000001), (n1 %% (1e3 || 1))))), gn23)
	try
		CRASH("A,b,c")
	catch (var/exception/xx75)
	try
		if (gradient(list(0, "#000", 1, "#fff", "space" = COLORSPACE_HSV), 2)) throw list(1, 2)
	catch (var/x76)
