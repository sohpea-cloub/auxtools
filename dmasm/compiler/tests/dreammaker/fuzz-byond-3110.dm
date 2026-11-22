// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 3110, "516.1659: new: result differs"
#define DEBUG

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
	var/vn6 = 7

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(156, /datum/fz/s5, "t153", 1)

/datum/fz/s5/proc/t153(a, b = 2, c)
	var/loops = 0
	var/n1 = -1
	var/s1 = "1"
	var/n2 = 65535
	var/list/L = list(0x1F, "abc", 65535)
	var/list/M = list("a" = 10, "b" = "Hello")
	var/datum/fz/o = new
	var/datum/fz/s5/p = new
	var/v
	L.Add(args.Copy(), "12abc")
	a = L.Find((10 == vn6))
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
