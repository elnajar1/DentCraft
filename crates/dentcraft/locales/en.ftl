# DentCraft interface text, English. This is the source language and the fallback for every
# other language: add each new message here first.
#
# Variables are snake_case (they come from the tr! macro). Tooth numbers arrive as text
# ($tooth, $a, $b), counts as numbers ($count).

## App bar

language-tip = Interface language

## Browser sections

section-case = Case
section-teeth = Teeth
section-material = Material
section-scans = Scans
section-workflow = Workflow
section-step = Step

## Case setup

case-setup-title = Case setup
case-setup-intro = Choose the teeth and their restorations, the material, and the role of each scan.
case-setup-start = Start design
case-setup-start-tip = Go to the design
case-setup-close = Close
case-setup-close-tip = Close the case setup; the toolbar opens it again
case-name-hint = Case name
teeth-units =
    { $count ->
        [one] { $count } unit
       *[other] { $count } units
    }
teeth-empty-hint = Click a tooth to add a restoration.

## Tooth chart

restoration-crown = Crown
restoration-coping = Coping
restoration-inlay = Inlay
restoration-onlay = Onlay
restoration-veneer = Veneer
restoration-pontic = Pontic
restoration-none = No restoration
chart-bridge = Bridge
# Patient's right and left, either side of the midline.
chart-side-right = R
chart-side-left = L
chart-tooth-hover = { $tooth } · { $kind }
chart-connect = Connect { $a }–{ $b } into a bridge
chart-disconnect = Disconnect { $a }–{ $b }
tooth-menu-title = Tooth { $tooth }
# $quadrant is the FDI quadrant (1–4), $tooth_type one of incisor, canine, premolar, molar.
tooth-description =
    { $quadrant ->
        [1] Upper right
        [2] Upper left
        [3] Lower left
       *[4] Lower right
    } { $tooth_type ->
        [incisor] incisor
        [canine] canine
        [premolar] premolar
       *[molar] molar
    }

## Case issues

issue-duplicate-tooth = Tooth { $tooth } has more than one restoration
issue-connector-not-adjacent = Connector { $a }–{ $b } joins teeth that are not neighbours
issue-connector-without-units = Connector { $a }–{ $b } needs a crown, coping or pontic on both teeth
issue-floating-pontic = Pontic { $tooth } is not connected to a neighbouring unit
issue-missing-jaw-scan =
    No scan for the { $arch ->
        [upper] upper
       *[lower] lower
    } arch, where tooth { $tooth } is restored

## Material

material-zirconia = Zirconia (monolithic)
material-lithium-disilicate = Lithium disilicate
material-pmma = PMMA (provisional)
material-wax = Wax / burnout resin
material-cement-gap = Cement gap
material-min-occlusal = Min occlusal
material-min-wall = Min wall
material-min-connector = Min connector

## Scans

scans-import-tip = Import scans…
scans-empty = No scans yet. Import or drop STL, PLY or OBJ files.
scan-colour = Colour
scan-remove = Remove from the scene
scan-hide = Hide
scan-show = Show
scan-loading = Loading…
scan-opacity = Opacity
# $count is already formatted with thousands separators.
scan-triangles = { $count } triangles
role-upper = Upper jaw
role-lower = Lower jaw
role-upper-pre-op = Upper jaw (pre-op)
role-lower-pre-op = Lower jaw (pre-op)
role-bite = Bite registration
role-other = Other
