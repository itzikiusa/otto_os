func emit(event Event) { producer.Publish("orders.created", event) }
