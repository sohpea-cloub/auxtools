// Guards: the code of a past finding: reload mode on 516.1688, seed 32186, "new: globals differ"
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

/obj/fz
	var/datum/fz/vo18

var/gn19 = 0.5
var/gt20 = "abc"
var/ga21 = /obj/fz
var/gn23 = 255
var/gn24 = 100

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()
	return "news=[fz_news] gn19=[fz_show(gn19, 2)] gt20=[fz_show(gt20, 2)] ga21=[fz_show(ga21, 2)] gn23=[fz_show(gn23, 2)] gn24=[fz_show(gn24, 2)]"

/proc/fz_intro()

/proc/fz_run()
	fz_case(68, /obj/fz, "Verb 62", 1)

/datum/fz/proc/h35(x1 = 2, x2, x3)

/obj/fz/proc/h47(x1 = 0.001, x2)
	gn24 = noise_hash((-(gn24)), (x1 || (-(gn23))))

/obj/fz/verb/vb61(a as num, b as num, c as text)
	set name = "Verb 62"
	var/n2 = 7
	var/list/L = list(-4, "UPPER lower", 16777215)
	var/list/M = list("a" = -4, "b" = "A,b,c")
	var/obj/fz/p = new
	if (call(p, "h47")(arglist(list(L, vo18?.h35((5 * n2), M.Copy(), "a b")))))
		return
