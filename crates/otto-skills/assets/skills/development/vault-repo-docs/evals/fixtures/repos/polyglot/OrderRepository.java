interface Orders extends JpaRepository<Order, String> {
    Optional<Order> findByCustomerId(String id);
}
