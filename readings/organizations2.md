# Auto.dev Tenancy

## Description

Auto.dev-specific tenancy binding: a Customer owns its Domains. This
file coexists with the metamodel's User → Organization → App → Domain
hierarchy rather than replacing it. When the compiler loads both the
metamodel and this file, Customer-to-Domain becomes an additional
lookup path alongside User-to-Organization-to-App-to-Domain.

## Cross-domain References

Customer (from customer-auth)
Domain (from metamodel organizations)

## Fact Types

### Domain

Customer has Domain.

## Constraints

Each Domain belongs to at most one Customer.
