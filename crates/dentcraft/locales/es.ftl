# DentCraft interface text, Spanish (neutral Latin American).
#
# Dental terminology reviewed by a practising dentist; see docs/i18n/glossary-es.md.
# Address form: usted. A message missing here is shown in English.

## App bar

language-tip = Idioma de la interfaz

## Browser sections

section-case = Caso
section-teeth = Dientes
section-material = Material
section-scans = Escaneos
section-workflow = Flujo de trabajo
section-step = Paso

## Case setup

case-setup-title = Configuración del caso
case-setup-intro = Elija los dientes y sus restauraciones, el material y el rol de cada escaneo.
case-setup-start = Iniciar diseño
case-setup-start-tip = Ir al diseño
case-setup-close = Cerrar
case-setup-close-tip = Cerrar la configuración del caso; la barra de herramientas la vuelve a abrir
case-name-hint = Nombre del caso
teeth-units =
    { $count ->
        [one] { $count } unidad
       *[other] { $count } unidades
    }
teeth-empty-hint = Haga clic en un diente para añadir una restauración.

## Tooth chart

restoration-crown = Corona
restoration-coping = Cofia
restoration-inlay = Inlay
restoration-onlay = Onlay
restoration-veneer = Carilla
restoration-pontic = Póntico
restoration-none = Sin restauración
chart-bridge = Puente
# Derecha e izquierda del paciente.
chart-side-right = D
chart-side-left = I
chart-tooth-hover = { $tooth } · { $kind }
chart-connect = Unir { $a }–{ $b } en un puente
chart-disconnect = Separar { $a }–{ $b }
tooth-menu-title = Diente { $tooth }
tooth-description =
    { $tooth_type ->
        [incisor] Incisivo
        [canine] Canino
        [premolar] Premolar
       *[molar] Molar
    } { $quadrant ->
        [1] superior derecho
        [2] superior izquierdo
        [3] inferior izquierdo
       *[4] inferior derecho
    }

## Case issues

issue-duplicate-tooth = El diente { $tooth } tiene más de una restauración
issue-connector-not-adjacent = El conector { $a }–{ $b } une dientes que no son contiguos
issue-connector-without-units = El conector { $a }–{ $b } necesita una corona, cofia o póntico en ambos dientes
issue-floating-pontic = El póntico { $tooth } no está unido a ninguna unidad vecina
issue-missing-jaw-scan =
    No hay escaneo de la arcada { $arch ->
        [upper] superior
       *[lower] inferior
    }, donde se restaura el diente { $tooth }

## Material

material-zirconia = Zirconio monolítico
material-lithium-disilicate = Disilicato de litio
material-pmma = PMMA (provisional)
material-wax = Cera / resina calcinable
material-cement-gap = Espacio para cemento
material-min-occlusal = Grosor oclusal mín.
material-min-wall = Grosor axial mín.
material-min-connector = Sección mín. de conector

## Scans

scans-import-tip = Importar escaneos…
scans-empty = Aún no hay escaneos. Importe o arrastre archivos STL, PLY u OBJ.
scan-colour = Color
scan-remove = Quitar de la escena
scan-hide = Ocultar
scan-show = Mostrar
scan-loading = Cargando…
scan-opacity = Opacidad
scan-triangles = { $count } triángulos
role-upper = Arcada superior
role-lower = Arcada inferior
role-upper-pre-op = Arcada superior (preoperatoria)
role-lower-pre-op = Arcada inferior (preoperatoria)
role-bite = Registro de mordida
role-other = Otro
