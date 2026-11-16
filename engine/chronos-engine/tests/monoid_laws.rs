use chronos_engine::{BoundedWord, Monoid};

type Word = BoundedWord<u8, 3>;

fn domain() -> Vec<Word> {
    let mut values = vec![Word::identity()];
    for length in 1..=3 {
        for bits in 0..(1 << length) {
            values.push(Word::from_items(
                (0..length).map(|shift| ((bits >> shift) & 1) as u8),
            ));
        }
    }
    values.push(Word::from_items([0, 1, 0, 1]));
    values.push(Word::Capacity);
    values
}

#[test]
fn bounded_words_form_a_closed_monoid() {
    let values = domain();
    for a in &values {
        assert_eq!(a.combine(&Word::identity()), *a, "right identity");
        assert_eq!(Word::identity().combine(a), *a, "left identity");

        for b in &values {
            for c in &values {
                assert_eq!(
                    a.combine(b).combine(c),
                    a.combine(&b.combine(c)),
                    "associativity"
                );
            }
        }
    }
}

#[test]
fn order_is_preserved_and_capacity_overflow_is_an_explicit_absorbing_value() {
    let left = Word::from_items([0, 1]);
    let right = Word::singleton(2);
    assert_eq!(left.combine(&right).as_slice(), Some(&[0, 1, 2][..]));
    assert_ne!(left.combine(&right), right.combine(&left));

    let overflow = Word::from_items([0, 1, 2, 3]);
    assert!(overflow.is_overflow());
    assert!(overflow.combine(&left).is_overflow());
    assert!(left.combine(&overflow).is_overflow());

    assert!(Word::Capacity.combine(&left).is_capacity_fault());
    assert!(left.combine(&Word::Capacity).is_capacity_fault());
}
