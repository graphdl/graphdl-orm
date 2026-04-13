# Tutor Configuration

App-level declarations that apply across every tutor domain: which
generators are opted in, and which nouns are federated through external
systems. The underscore prefix keeps this file at the top of the
domains/ directory listing.

## Entity Types

App(.Slug) is an entity type.
Generator(.Name) is an entity type.

## Fact Types

App uses Generator.

## Instance Facts

App 'tutor' uses Generator 'sqlite'.
App 'tutor' uses Generator 'solidity'.
App 'tutor' uses Generator 'ilayer'.

### Federation

User is backed by External System 'auth.vin'.
Stripe Customer is backed by External System 'stripe'.

These declarations live in the bundled metamodel's organizations.md.
This file surfaces them for tutor learners so that federation works out
of the box. No credentials are checked in. Set AREST_SECRET_STRIPE
and AREST_SECRET_AUTH_VIN in your environment if you want live fetches.
