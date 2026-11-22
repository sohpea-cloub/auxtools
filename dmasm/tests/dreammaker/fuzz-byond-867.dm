// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 867, "515.1637: new: result differs"
#define DEBUG

#define FZD1 ""

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

/mob/fz
	var/list/vl21 = list()

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(102, /mob/fz/s5, "t92", 1)

/mob/fz/s5/s6/proc/h37(x1, x2 = 10, x3)

/mob/fz/s5/proc/t92(a, b = 2, c)
	var/loops = 0
	var/n1 = 5
	var/s1 = "Hello"
	var/n2 = 256
	var/list/L = list(1e3, "abc", 0)
	var/list/M = list("a" = 7, "b" = "Hello")
	var/mob/fz/s5/s6/o = new /mob/fz/s5/s6(null, 1e3, "a b")
	var/mob/fz/s5/p = new /mob/fz/s5(null, 16777215, "A,b,c")
	var/v
	for (var/i97 in 0 to ispath(src.vl21[FZD1], /datum/fz))
		if (++loops > 40) break
		try
			var/xz100 = 1 / (loops * 0)
		catch (var/exception/xx100)
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
