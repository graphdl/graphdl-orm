# Content Management

Exercises: rich value types, frequency constraints, set comparison (XO/OR),
mandatory roles, 1:1 relationships, symmetric/asymmetric ring constraints,
derivation rules with CWA/OWA, multiple constraint modalities.

## Entity Types

Article(.Slug) is an entity type.
Author(.Handle) is an entity type.
MediaAsset(.Asset Id) is an entity type.

## Value Types

Slug is a value type.
Handle is a value type.
Asset Id is a value type.
Headline is a value type.
Excerpt is a value type.
Content Body is a value type.
Publish Date is a value type.
Word Count is a value type.
Media Type is a value type.
  The possible values of Media Type are 'image', 'video', 'audio', 'document'.
File URL is a value type.
Alt Text is a value type.
Bio is a value type.
Content Status is a value type.
  The possible values of Content Status are 'draft', 'review', 'published', 'archived'.

## Readings

### Author

Author has Display Name.
  Each Author has exactly one Display Name.

Author has Bio.
  Each Author has at most one Bio.

Author has Avatar URL.
  Each Author has at most one Avatar URL.

### Article

Article has Headline.
  Each Article has exactly one Headline.

Article has Excerpt.
  Each Article has at most one Excerpt.

Article has Content Body.
  Each Article has at most one Content Body.

Article has Publish Date.
  Each Article has at most one Publish Date.

Article has Word Count.
  Each Article has at most one Word Count.

Article has Content Status.
  Each Article has exactly one Content Status.

Article is written by Author.
  Each Article is written by exactly one Author.
  It is possible that some Author writes more than one Article.

Article features MediaAsset.
  It is possible that some Article features more than one MediaAsset.
  In each population of Article features MediaAsset, each Article, MediaAsset combination occurs at most once.

Article is related to Article.
  In each population of Article is related to Article, each Article, Article combination occurs at most once.
  No Article is related to itself.
  If Article 1 is related to Article 2, then Article 2 is related to Article 1.

### Media Asset

MediaAsset has File URL.
  Each MediaAsset has exactly one File URL.

MediaAsset has Media Type.
  Each MediaAsset has exactly one Media Type.

MediaAsset has Alt Text.
  Each MediaAsset has at most one Alt Text.

## Constraints

If some Article1 is related to some Article2 then that Article2 is related to that Article1. (symmetric)

It is obligatory that each Article has Content Body before Content Status is 'published'.

## Derivation Rules

-- Derivation rule decomposed into elementary premises. Each "and"-joined clause
-- is itself an elementary fact type reading, not prose.
* Article has Word Count iff Article has Content Body
  and Word Count is the count of words in that Content Body.

## Instance Facts

### Article State Machine

State Machine Definition 'Article' is for Noun 'Article'.
Status 'Draft' is defined in State Machine Definition 'Article'.
Status 'In Review' is defined in State Machine Definition 'Article'.
Status 'Published' is defined in State Machine Definition 'Article'.
Status 'Archived' is defined in State Machine Definition 'Article'.
Status 'Draft' is initial.

Transition 'submit' is from Status 'Draft'.
Transition 'submit' is to Status 'In Review'.
Transition 'submit' is triggered by Event Type 'submit'.

Transition 'publish' is from Status 'In Review'.
Transition 'publish' is to Status 'Published'.
Transition 'publish' is triggered by Event Type 'publish'.

Transition 'revise' is from Status 'In Review'.
Transition 'revise' is to Status 'Draft'.
Transition 'revise' is triggered by Event Type 'revise'.

Transition 'archive' is from Status 'Published'.
Transition 'archive' is to Status 'Archived'.
Transition 'archive' is triggered by Event Type 'archive'.

Transition 'unarchive' is from Status 'Archived'.
Transition 'unarchive' is to Status 'Draft'.
Transition 'unarchive' is triggered by Event Type 'unarchive'.

Domain 'content' has Visibility 'public'.
