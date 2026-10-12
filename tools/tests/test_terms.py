from utter_tools.text import candidate_terms, lowercase_counts, relevant_terms


def test_relevant_terms_are_the_ones_heard():
    # The same cases as gate::relevant_terms in crates/utter-pipeline/src/gate.rs.
    d = ["Supabase", "Kubernetes", "Nguyen", "UMAP"]
    assert relevant_terms("we moved the auth to super base and kubernetes", d) == [
        "Supabase",
        "Kubernetes",
    ]
    assert relevant_terms("lunch at noon?", d) == []
    assert relevant_terms("go to the top and then to the beach", ["TOP", "Sand", "CTO"]) == ["TOP"]


def test_candidate_terms_are_terms_and_names_not_everyday_words():
    texts = ["We run Kubernetes on the GPU nodes with Safeway data.", "the key is on the left"]
    lower = lowercase_counts(texts)
    assert candidate_terms(texts[0], lower) == {"GPU", "Kubernetes", "Safeway"}
    # Names are capitalized mid-sentence; acronyms and mixed-case terms count either way.
    assert candidate_terms(texts[0], lower, names=False) == {"GPU"}
