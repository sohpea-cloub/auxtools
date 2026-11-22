// Guards: the code of a reduced fuzz case: byond mode on 516.1689, seed 12248, "516.1689: the world gave no results (1): BUG: Crashing due to an illegal operation!"
// Known: /datum/fz/s8/s9/proc/t139  `continue` to a labelled block holding a do-while in a counted for loop: DreamMaker pops the for loop's range first and the server crashes (a BYOND bug, which dmasm warns of); dmasm leaves the range
#define DEBUG

#define FZD5 -3.25
#define FZD7 -1
var/list/fz_tr = list()

/proc/fz_ovs(v)

/proc/fz_ovt(v)

/proc/fz_h(label, text)

/proc/fz_chk(label, got, want)

/proc/fz_err(e)

/obj/fzo
/datum/fzo/o2/proc/operator~()
	return src
var/fz_ck = 0

/proc/fzc_fail(k)
	return "r[k]"

/proc/fzc_ref(k)

/proc/fzc_g(k)

/datum/fzc
var/fzm_refused = 0

/proc/fzm_lab(atom/A)
	return istype(A) ? A.name : "?"

/proc/fzm_log(atom/A, what, O)

/proc/fzm_at(atom/movable/M)

/mob/fzm/Bump(atom/A)

/turf/fzm
	var/refuse = ""
/area/fzm
	var/allow = ""

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

var/ga26
var/gn27 = 0x1F
var/list/gl28 = list(1, 2)
var/list/gl29 = list(1, 2)
var/gt30 = "[gn27]"

/proc/fz_setup()

/proc/fz_fields(datum/d)
	return ""

/proc/fz_globals()
	return "news=[fz_news] ga26=[fz_show(ga26, 2)] gn27=[fz_show(gn27, 2)] gl28=[fz_show(gl28, 2)] gl29=[fz_show(gl29, 2)] gt30=[fz_show(gt30, 2)]"

/proc/fz_intro()
	fz_news = 0

/proc/fz_run()
	fz_case(147, /datum/fz/s8/s9, "t139", 1)

/obj/fz/s10/proc/h35(x1, x2)

/obj/fz/proc/h38(x1 = "k=1&j=2", x2 = 255, x3 = null)
	var/v
	var/list/xl39 = list("c", "a")
	try
		xl39.Add("b")
		xl39.len = 0
	catch (var/xe39)

/obj/fz/s11/proc/h47(x1, x2)

/mob/fz/verb/vb94(a as num, b as num, c as text)
	var/loops = 0
	var/n1 = 2.5
	var/s1 = "a b"
	var/n2 = 0x1F
	var/list/L = list(7, "abc", 2.5)
	var/list/M = list("a" = 65535, "b" = "UPPER lower")
	var/mob/fz/o = new
	var/mob/fz/p = new /mob/fz
	var/v
	switch (global.gn27)
		if (100)
			if ("\Roman[FZD5]")
				var/xt98 = "naïve café"
				try
					fz_chk("ck unichar copy-back", "[copytext_char(xt98, -2)]", "fé")
					fz_h("unichar copy-back", "[copytext_char(xt98, -2)]")
				catch (var/xe98)
					fz_h("unichar copy-back", fz_err(xe98))
				var/list/xl99 = list("a", null, 2.5, "a", 1)
				try
					xl99 += 3
				catch (var/xe99)
	var/xt101 = "Ωmega ΩΩ"
	try
		fz_chk("ck unichar copy-bytes", "[copytext(xt101, 12, 12)]", "")
	catch (var/xe101)
	try
		var/datum/xo102 = new /datum/fzo/o2(6)
	catch (var/xe102)
	var/list/xr103 = list()
	try
		if (1)
		if (1)
			var/list/xl103_2 = list()
	catch (var/xu103)
	var/turf/fzm/xs104 = locate(2, 2, 1)
	var/mob/fzm/xm104 = new(xs104)
	var/turf/fzm/xd104 = locate(1, 5, 1)
	fz_tr.Cut()
	fzm_refused = 0
	try
		var/xr104 = xm104.Move(xd104)
	catch (var/xe104)
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)

/datum/fz/s8/s9/proc/t139(a, b = 2, c)
	var/loops = 0
	var/n1 = -3.25
	var/s1 = "3.5"
	var/n2 = 0.001
	var/list/L = list(7, "Hello", 255)
	var/list/M = list("a" = 1e3, "b" = "  pad  ")
	var/obj/fz/o = new(null, 3, "3.5")
	var/obj/fz/s10/p = new /obj/fz/s10(null, 0, "12abc")
	var/v
	fz_ck = 9
	var/xw142 = fzc_ref(9)
	try
		var/xr142 = fzc_g(9)
		fz_chk("ck call global assert", "ok [fz_ovs(xr142)]", xw142)
	catch (var/xe142)
	switch ((p.h35(x2 = (min(max(2, -16777216), 16777215) ^ min(max(FZD7, -16777216), 16777215))) - (-(FZD7))))
		if (20 to 30)
			if (isarea(gl29)) return text2path("/datum/fz")
		if (-1)
			var/list/xr144 = list()
			try
				lf144_1:
					for (var/xi144_2 in 1 to 3)
						xr144 += "1"
						lf144_3:
							var/xi144_4 = 0
							do
								xi144_4++
								if (1)
									continue lf144_3
							while (xi144_4 < 2)
				xr144 += "6"
			catch (var/xu144)
	ga26 = (0.5 && n1)
	var/list/xr145 = list()
	try
		var/xi145_1 = 0
		do
			if (0)
				xr145 += "4"
		while (xi145_1 < 3)
		if (0)
			xr145 += "5"
			var/xi145_3 = 0
		try
			if (0)
		catch (var/xc145_6)
			xr145 += "14"
			var/xi145_7 = 0
	catch (var/xu145)
	if (gn27)
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
