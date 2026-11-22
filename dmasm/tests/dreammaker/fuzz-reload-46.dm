// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 46, "new: result differs"
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

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(106, /mob/fz, "Verb 102", 0)

/datum/fz/s3/proc/h36(x1 = "3.5", x2)

/obj/fz/proc/t46(a, b = 2, c)
	var/loops = 0
	var/n1 = -4
	var/s1 = ""
	var/n2 = 0.5
	var/list/L = list(-1, "a\tb", -3.25)
	var/list/M = list("a" = 0.5, "b" = "a b")
	var/datum/fz/s3/o = new
	var/datum/fz/s3/p = new /datum/fz/s3
	var/v

/mob/fz/verb/vb101(a as num, b as num, c as text)
	set name = "Verb 102"
	var/loops = 0
	var/n1 = 2
	var/s1 = "abc"
	var/n2 = 100
	var/list/L = list(0x1F, "  pad  ", 0.5)
	var/list/M = list("a" = -3.25, "b" = "3.5")
	var/mob/fz/o = new
	var/datum/fz/p = new
	var/v
	a = length_char("ÀBçd")
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
