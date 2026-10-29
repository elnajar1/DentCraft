//! Published average crown dimensions of the permanent teeth.
//!
//! All values are millimetres, from the "Measurements of the teeth" table of average dimensions
//! in: S. J. Nelson, M. M. Ash, *Wheeler's Dental Anatomy, Physiology and Occlusion*, 9th ed.,
//! Saunders, 2010. Clean-room note: these are published textbook averages of public dental
//! literature, not data taken from any dental CAD product.
//!
//! Per tooth the order is: crown length (mid-facial cervical line to the highest point),
//! mesiodistal diameter of the crown, mesiodistal diameter at the cervix, buccolingual
//! (labiolingual) diameter, buccolingual diameter at the cervix, curvature of the cervical line
//! mesially, and distally (how far the CEJ reaches towards the occlusal).

use dentcraft_case::{Arch, Fdi};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dimensions {
    pub crown_length: f64,
    pub mesiodistal: f64,
    pub mesiodistal_cervix: f64,
    pub buccolingual: f64,
    pub buccolingual_cervix: f64,
    pub cej_mesial: f64,
    pub cej_distal: f64,
}

/// Upper right quadrant, positions 1 (central incisor) to 8 (third molar).
const UPPER: [[f64; 7]; 8] = [
    [10.5, 8.5, 7.0, 7.0, 6.0, 3.5, 2.5],   // 11
    [9.0, 6.5, 5.0, 6.0, 5.0, 3.0, 2.0],    // 12
    [10.0, 7.5, 5.5, 8.0, 7.0, 2.5, 1.5],   // 13
    [8.5, 7.0, 5.0, 9.0, 8.0, 1.0, 0.0],    // 14
    [8.5, 7.0, 5.0, 9.0, 8.0, 1.0, 0.0],    // 15
    [7.5, 10.0, 8.0, 11.0, 10.0, 1.0, 0.0], // 16
    [7.0, 9.0, 7.0, 11.0, 10.0, 1.0, 0.0],  // 17
    [6.5, 8.5, 6.5, 10.0, 9.5, 1.0, 0.0],   // 18
];

/// Lower right quadrant, positions 1 to 8.
const LOWER: [[f64; 7]; 8] = [
    [9.0, 5.0, 3.5, 6.0, 5.3, 3.0, 2.0],   // 41
    [9.5, 5.5, 4.0, 6.5, 5.8, 3.0, 2.0],   // 42
    [11.0, 7.0, 5.5, 7.5, 7.0, 2.5, 1.0],  // 43
    [8.5, 7.0, 5.0, 7.5, 6.5, 1.0, 0.0],   // 44
    [8.0, 7.0, 5.0, 8.0, 7.0, 1.0, 0.0],   // 45
    [7.5, 11.0, 9.0, 10.5, 9.0, 1.0, 0.0], // 46
    [7.0, 10.5, 8.0, 10.0, 9.0, 1.0, 0.0], // 47
    [7.0, 10.0, 7.5, 9.5, 9.0, 1.0, 0.0],  // 48
];

pub(crate) fn dimensions(fdi: Fdi) -> Dimensions {
    let index = usize::from(fdi.position() - 1);
    let row = if fdi.arch() == Arch::Upper {
        UPPER[index]
    } else {
        LOWER[index]
    };
    Dimensions {
        crown_length: row[0],
        mesiodistal: row[1],
        mesiodistal_cervix: row[2],
        buccolingual: row[3],
        buccolingual_cervix: row[4],
        cej_mesial: row[5],
        cej_distal: row[6],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lookup_follows_the_quadrant() {
        let d16 = dimensions(Fdi::new(16).unwrap());
        assert!((d16.crown_length - 7.5).abs() < 1e-12);
        assert!((d16.mesiodistal - 10.0).abs() < 1e-12);
        let d31 = dimensions(Fdi::new(31).unwrap());
        assert!((d31.crown_length - 9.0).abs() < 1e-12);
        assert!((d31.cej_mesial - 3.0).abs() < 1e-12);
        // Left teeth use the same averages as their right counterparts.
        assert_eq!(
            dimensions(Fdi::new(23).unwrap()),
            dimensions(Fdi::new(13).unwrap())
        );
    }

    #[test]
    fn every_measurement_is_positive() {
        for fdi in Fdi::all() {
            let d = dimensions(fdi);
            assert!(d.crown_length > 3.0 && d.crown_length < 13.0);
            assert!(d.mesiodistal > 3.0 && d.mesiodistal < 12.0);
            assert!(d.buccolingual > 4.0 && d.buccolingual < 12.0);
            assert!(d.mesiodistal_cervix < d.mesiodistal);
            assert!(d.buccolingual_cervix < d.buccolingual);
            assert!(d.cej_mesial >= d.cej_distal);
        }
    }
}
