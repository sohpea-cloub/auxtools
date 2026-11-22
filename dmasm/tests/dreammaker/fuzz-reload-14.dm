// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 14, "new: result differs"
#define DEBUG

#define FZD2 7
#define FZD4 2

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
	var/vn7

/datum/fz/s5

var/gn22 = 5

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(118, /obj/fz, "t116", 1)

/obj/fz/proc/t116(a, b = 2, c)
	var/loops = 0
	var/n1 = 0x1F
	var/s1 = ""
	var/n2 = 256
	var/list/L = list(7, "a\tb", 1)
	var/list/M = list("a" = 5, "b" = "12abc")
	var/datum/fz/o = new(0.5, "1")
	var/datum/fz/s5/p = new /datum/fz/s5(-4, "UPPER lower")
	var/v
	try
		try
			gn22 = n2 = (v ? 16777215 : FZD4)
		catch (var/x131)
	catch (var/x129)
	s1 = (L[1] ? jointext(splittext_char("é", ""), "|") : ("([p.vn7]\th  \n([FZD2]\t" + "  pad  "))
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
